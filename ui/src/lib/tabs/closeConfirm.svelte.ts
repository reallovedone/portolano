import { t } from "../i18n/strings";
import type { TabsStore } from "./store.svelte";

/**
 * Asks before closing tabs that hold connected sessions (SSH/terminal, RDP, VNC). Web tabs and
 * sessions that are connecting, failed or already gone never count (`TabsStore.liveCount`).
 *
 * One confirmation can be open at a time. A request made while one is open is refused: it
 * answers `false` at once and no second dialog appears; the open one is left alone and still
 * gets the user's answer. (Answering "yes" to a question the user did not just read would be
 * worse: a tab-close dialog must never approve an app quit.)
 *
 * The count in the dialog is fixed when it opens. If a session drops meanwhile, "Close" still
 * closes everything asked for without error.
 */
export class CloseConfirmStore {
  /** The open confirmation, or null. Rendered by `App.svelte`. */
  pending = $state<{ count: number } | null>(null);
  #resolve: ((ok: boolean) => void) | null = null;
  readonly #tabs: TabsStore;

  constructor(tabs: TabsStore) {
    this.#tabs = tabs;
  }

  /** Dialog texts for the open confirmation. */
  get title(): string {
    const count = this.pending?.count ?? 0;
    return count === 1 ? t("closeConfirm.titleOne") : t("closeConfirm.titleMany", { count });
  }

  /**
   * Resolves `true` at once when none of `tabIds` has a connected session; otherwise opens the
   * confirmation and resolves with the user's choice (`false` for Cancel and Esc). Callers
   * (tab close, update install, app quit) pass the tabs they are about to close.
   */
  confirmClose(tabIds: string[]): Promise<boolean> {
    const count = this.#tabs.liveCount(tabIds);
    if (count === 0) return Promise.resolve(true);
    if (this.pending) return Promise.resolve(false);
    this.pending = { count };
    return new Promise<boolean>((resolve) => {
      this.#resolve = resolve;
    });
  }

  /** The user's answer to the open confirmation. */
  answer(ok: boolean): void {
    const resolve = this.#resolve;
    this.#resolve = null;
    this.pending = null;
    resolve?.(ok);
  }

  async closeTab(tabId: string): Promise<void> {
    if (await this.confirmClose([tabId])) await this.#tabs.close(tabId);
  }

  // Only the tabs that were counted and confirmed are closed: a tab opened while the dialog
  // was up stays open.
  async closeOthers(keepId: string): Promise<void> {
    const others = this.#tabs.tabs.filter((x) => x.id !== keepId).map((x) => x.id);
    if (await this.confirmClose(others)) await this.#closeIds(others);
  }

  async closeAll(): Promise<void> {
    const all = this.#tabs.tabs.map((x) => x.id);
    if (await this.confirmClose(all)) await this.#closeIds(all);
  }

  async #closeIds(ids: string[]): Promise<void> {
    for (const id of ids) await this.#tabs.close(id);
  }
}
