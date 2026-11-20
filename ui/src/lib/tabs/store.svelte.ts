import { SvelteSet } from "svelte/reactivity";

import { toIpcError, type AppError, type Ipc, type SessionId, type Uuid } from "../ipc";
import { t } from "../i18n/strings";
import type { DocumentStore } from "../state/document.svelte";
import type { NoticeStore } from "../state/notices.svelte";
import type { VaultStore } from "../state/vault.svelte";
import { effectiveCredentialOf, isConnection } from "../tree/model";
import {
  activateTab,
  addTab,
  cycleTab,
  emptyTabs,
  isConnectionTab,
  moveTab,
  newTab,
  openHomeTab,
  removeOtherTabs,
  removeTab,
  removeTabsWhere,
  sessionFailed,
  sessionOpened,
  sessionRetrying,
  type ConnectionTab,
  type Tab,
  type TabsState,
} from "./model";
import { liveAfter } from "./liveness";
import {
  DEFAULT_GEOMETRY,
  SessionBridge,
  openSession,
  type SessionEvent,
  type SessionGeometry,
} from "./session";

/** Tabs and their backend sessions: the IPC side of the pure transitions in `model.ts`. */
export class TabsStore {
  state = $state<TabsState>(emptyTabs);
  geometry: SessionGeometry = DEFAULT_GEOMETRY;
  readonly #bridges = new Map<string, SessionBridge>();
  /** Tabs whose credential is typed by hand for this session (the vault is skipped). */
  readonly #manual = new Set<string>();
  /** Tabs whose session is connected (see `liveness.ts`). */
  readonly #live = new SvelteSet<string>();
  readonly #ipc: Ipc;
  readonly #doc: DocumentStore;
  readonly #vault: VaultStore;
  readonly #notices: NoticeStore;

  constructor(ipc: Ipc, doc: DocumentStore, vault: VaultStore, notices: NoticeStore) {
    this.#ipc = ipc;
    this.#doc = doc;
    this.#vault = vault;
    this.#notices = notices;
  }

  get tabs(): Tab[] {
    return this.state.tabs;
  }

  /** The tabs that hold a connection (everything but Home). */
  get connectionTabs(): ConnectionTab[] {
    return this.state.tabs.filter(isConnectionTab);
  }

  /** The active tab when it holds a connection; null for Home or no tab. */
  get activeConnection(): ConnectionTab | null {
    const tab = this.activeTab;
    return isConnectionTab(tab) ? tab : null;
  }

  get activeTab(): Tab | null {
    return this.state.tabs.find((tab) => tab.id === this.state.activeId) ?? null;
  }

  /** Live title: follows renames of the connection. */
  title(tab: Tab): string {
    if (tab.type === "home") return t("tabs.home");
    return this.#doc.node(tab.connectionId)?.name ?? tab.title;
  }

  bridge(tabId: string): SessionBridge {
    let bridge = this.#bridges.get(tabId);
    if (!bridge) {
      bridge = new SessionBridge();
      this.#bridges.set(tabId, bridge);
    }
    return bridge;
  }

  /** True while the tab's session is connected (not connecting, not waiting for a prompt). */
  isLive(tabId: string): boolean {
    return this.#live.has(tabId);
  }

  /** How many of the given tabs (default: all) have a connected session. */
  liveCount(tabIds?: string[]): number {
    const ids = tabIds ?? this.state.tabs.map((tab) => tab.id);
    return ids.filter((id) => this.#live.has(id)).length;
  }

  #observe(tabId: string, attempt: number, kind: ConnectionTab["kind"], event: SessionEvent): void {
    // Events of a closed or retried attempt must not change the tab's state.
    if (!this.#isCurrent(tabId, attempt)) return;
    // A failed tab keeps its attempt number: a late event must not revive it.
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab) || tab.status === "failed") return;
    const next = liveAfter(kind, this.#live.has(tabId), event);
    if (next) this.#live.add(tabId);
    else this.#live.delete(tabId);
  }

  setGeometry(geometry: SessionGeometry): void {
    this.geometry = geometry;
  }

  /**
   * Opens a tab for a connection. If its effective credential is a vault credential and the
   * vault is locked, the unlock screen is shown first, with "enter manually for this session"
   * as the alternative; cancelling it opens nothing. A connection without a credential, or
   * whose credential cannot exist because no vault was ever created, needs no unlock and no
   * vault: the session asks for the credential itself. Returns the tab id, or null when
   * nothing was opened.
   */
  async openConnection(connectionId: Uuid): Promise<string | null> {
    const node = this.#doc.node(connectionId);
    if (!node || !isConnection(node)) return null;

    let manual = false;
    const effective = effectiveCredentialOf(this.#doc.nodes, connectionId);
    // `notCreated`: the reference is dangling, which the backend treats as "no credential".
    if (effective.kind === "id" && !this.#vault.unlocked && this.#vault.status !== "notCreated") {
      const outcome = await this.#vault.ensureUnlockedOrManual(node.name);
      if (outcome === "cancelled") return null;
      manual = outcome === "manual";
    }

    const tab = newTab(crypto.randomUUID(), node.id, node.name, node.options.type);
    if (manual) this.#manual.add(tab.id);
    this.state = addTab(this.state, tab);
    await this.#open(tab.id);
    return tab.id;
  }

  async #open(tabId: string): Promise<void> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab)) return;
    const attemptNo = tab.attempt;
    const node = this.#doc.node(tab.connectionId);
    if (!node || !isConnection(node)) {
      this.state = sessionFailed(this.state, tabId, {
        code: "not_found",
        message: t("tabs.connectionGone"),
      });
      return;
    }

    this.#live.delete(tabId);
    const bridge = new SessionBridge((event) => this.#observe(tabId, attemptNo, tab.kind, event));
    this.#bridges.set(tabId, bridge);
    let sessionId: SessionId;
    try {
      sessionId = await openSession(this.#ipc, node, bridge, this.geometry, {
        askCredentials: this.#manual.has(tabId),
      });
    } catch (e) {
      const error = toIpcError(e);
      if (this.#isCurrent(tabId, attemptNo)) {
        this.#live.delete(tabId);
        this.state = sessionFailed(this.state, tabId, { code: error.code, message: error.message });
      }
      return;
    }

    if (!this.#isCurrent(tabId, attemptNo)) {
      // The tab was closed (or retried) while the session was opening: do not leak it.
      await this.#closeSessions([sessionId]);
      return;
    }
    this.state = sessionOpened(this.state, tabId, sessionId);
  }

  #isCurrent(tabId: string, attempt: number): boolean {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    return isConnectionTab(tab) && tab.attempt === attempt;
  }

  async #closeSessions(ids: SessionId[]): Promise<void> {
    for (const id of ids) {
      try {
        await this.#ipc.sessionClose(id);
      } catch (e) {
        const error = toIpcError(e);
        // `unsupported` (mock) and `not_found` (already gone) are not worth a notice.
        if (error.code !== "unsupported" && error.code !== "not_found") {
          this.#notices.push("error", t("tabs.closeFailed", { message: error.message }));
        }
      }
    }
  }

  #sessionIds(tabs: Tab[]): SessionId[] {
    return tabs.flatMap((tab) => (tab.type === "connection" && tab.sessionId ? [tab.sessionId] : []));
  }

  #forget(tabs: Tab[]): void {
    for (const tab of tabs) {
      this.#bridges.delete(tab.id);
      this.#manual.delete(tab.id);
      this.#live.delete(tab.id);
    }
  }

  async close(tabId: string): Promise<void> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!tab) return;
    this.state = removeTab(this.state, tabId);
    this.#forget([tab]);
    await this.#closeSessions(this.#sessionIds([tab]));
  }

  async closeOthers(keepId: string): Promise<void> {
    const closing = this.state.tabs.filter((x) => x.id !== keepId);
    this.state = removeOtherTabs(this.state, keepId);
    this.#forget(closing);
    await this.#closeSessions(this.#sessionIds(closing));
  }

  async closeAll(): Promise<void> {
    const closing = this.state.tabs;
    this.state = emptyTabs;
    this.#forget(closing);
    await this.#closeSessions(this.#sessionIds(closing));
  }

  /** Closes the tabs of deleted connections. */
  async closeForConnections(ids: ReadonlySet<Uuid>): Promise<void> {
    const closing = this.state.tabs.filter((x) => x.type === "connection" && ids.has(x.connectionId));
    if (closing.length === 0) return;
    this.state = removeTabsWhere(this.state, (x) => x.type === "connection" && ids.has(x.connectionId));
    this.#forget(closing);
    await this.#closeSessions(this.#sessionIds(closing));
  }

  async retry(tabId: string): Promise<void> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab) || tab.status === "opening") return;
    const stale = this.#sessionIds([tab]);
    this.#live.delete(tabId);
    this.state = sessionRetrying(this.state, tabId);
    await this.#closeSessions(stale);
    await this.#open(tabId);
  }

  /**
   * Starts the tab's session again, whatever its state: a live session is closed first (the
   * backend never keeps the old one), a failed tab simply retries. Ignored while the tab is
   * already opening, so repeated clicks open one session.
   */
  async reconnect(tabId: string): Promise<void> {
    await this.retry(tabId);
  }

  /**
   * Opens another tab on the same connection, right after this one. The credential is looked up
   * again as for any new tab (a vault unlock may be asked). Returns the new tab id, or null.
   */
  async duplicate(tabId: string): Promise<string | null> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab)) return null;
    // New tabs open after the active one: make this one the active one first.
    this.state = activateTab(this.state, tabId);
    return this.openConnection(tab.connectionId);
  }

  /** For a tab that failed with `vault_locked`: unlock first, then retry. */
  async unlockAndRetry(tabId: string): Promise<void> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab)) return;
    if (await this.#vault.ensureUnlocked(this.title(tab))) await this.retry(tabId);
  }

  /** For a tab that failed with `vault_locked`: type the credential by hand instead. */
  async enterManuallyAndRetry(tabId: string): Promise<void> {
    if (!isConnectionTab(this.state.tabs.find((x) => x.id === tabId))) return;
    this.#manual.add(tabId);
    await this.retry(tabId);
  }

  /** A session component reports a fatal error after the session was opened. */
  async fail(tabId: string, error: AppError): Promise<void> {
    const tab = this.state.tabs.find((x) => x.id === tabId);
    if (!isConnectionTab(tab)) return;
    this.#live.delete(tabId);
    this.state = sessionFailed(this.state, tabId, error);
    await this.#closeSessions(this.#sessionIds([tab]));
  }

  /** Opens the Home tab after the active one; if it is already open, activates it. */
  openHome(): void {
    this.state = openHomeTab(this.state, crypto.randomUUID());
  }

  activate(tabId: string): void {
    this.state = activateTab(this.state, tabId);
  }

  next(): void {
    this.state = cycleTab(this.state, 1);
  }

  previous(): void {
    this.state = cycleTab(this.state, -1);
  }

  move(from: number, to: number): void {
    this.state = moveTab(this.state, from, to);
  }
}
