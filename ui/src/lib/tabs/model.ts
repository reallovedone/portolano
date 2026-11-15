// Pure tab-strip state transitions. The session lifecycle (IPC) lives in `store.svelte.ts`.

import type { AppError, SessionId, Uuid } from "../ipc";
import type { ConnectionKind } from "../tree/model";

export type TabStatus = "opening" | "open" | "failed";

export interface Tab {
  /** Identifies the tab (not the backend session; one connection can have several tabs). */
  id: string;
  connectionId: Uuid;
  title: string;
  kind: ConnectionKind;
  status: TabStatus;
  sessionId: SessionId | null;
  error: AppError | null;
  /** Bumped by a retry so session components are recreated. */
  attempt: number;
}

export interface TabsState {
  tabs: Tab[];
  activeId: string | null;
}

export const emptyTabs: TabsState = { tabs: [], activeId: null };

export function addTab(state: TabsState, tab: Tab): TabsState {
  const index = state.tabs.findIndex((t) => t.id === state.activeId);
  // New tabs open right after the active one.
  const at = index >= 0 ? index + 1 : state.tabs.length;
  const tabs = [...state.tabs.slice(0, at), tab, ...state.tabs.slice(at)];
  return { tabs, activeId: tab.id };
}

export function activateTab(state: TabsState, id: string): TabsState {
  return state.tabs.some((t) => t.id === id) ? { ...state, activeId: id } : state;
}

/** Closes a tab; if it was active, the next tab (or else the previous one) becomes active. */
export function removeTab(state: TabsState, id: string): TabsState {
  const index = state.tabs.findIndex((t) => t.id === id);
  if (index < 0) return state;
  const tabs = state.tabs.filter((t) => t.id !== id);
  let activeId = state.activeId;
  if (state.activeId === id) activeId = (tabs[index] ?? tabs[index - 1])?.id ?? null;
  return { tabs, activeId };
}

export function removeOtherTabs(state: TabsState, keepId: string): TabsState {
  const keep = state.tabs.find((t) => t.id === keepId);
  return keep ? { tabs: [keep], activeId: keepId } : state;
}

export function removeTabsWhere(state: TabsState, predicate: (tab: Tab) => boolean): TabsState {
  const tabs = state.tabs.filter((t) => !predicate(t));
  if (tabs.length === state.tabs.length) return state;
  const activeStillThere = tabs.some((t) => t.id === state.activeId);
  return { tabs, activeId: activeStillThere ? state.activeId : (tabs[tabs.length - 1]?.id ?? null) };
}

/** Moves the tab at `from` so that it ends up at index `to` (both clamped to the strip). */
export function moveTab(state: TabsState, from: number, to: number): TabsState {
  const last = state.tabs.length - 1;
  if (from < 0 || from > last) return state;
  const target = Math.max(0, Math.min(last, to));
  if (target === from) return state;
  const tabs = state.tabs.slice();
  const [moved] = tabs.splice(from, 1);
  if (!moved) return state;
  tabs.splice(target, 0, moved);
  return { ...state, tabs };
}

/** Next/previous tab with wrap-around. */
export function cycleTab(state: TabsState, step: 1 | -1): TabsState {
  if (state.tabs.length === 0) return state;
  const index = state.tabs.findIndex((t) => t.id === state.activeId);
  const next = (index + step + state.tabs.length) % state.tabs.length;
  const tab = state.tabs[next];
  return tab ? { ...state, activeId: tab.id } : state;
}

export function patchTab(state: TabsState, id: string, patch: Partial<Tab>): TabsState {
  if (!state.tabs.some((t) => t.id === id)) return state;
  return { ...state, tabs: state.tabs.map((t) => (t.id === id ? { ...t, ...patch } : t)) };
}

export function sessionOpened(state: TabsState, id: string, sessionId: SessionId): TabsState {
  return patchTab(state, id, { status: "open", sessionId, error: null });
}

export function sessionFailed(state: TabsState, id: string, error: AppError): TabsState {
  return patchTab(state, id, { status: "failed", sessionId: null, error });
}

export function sessionRetrying(state: TabsState, id: string): TabsState {
  const tab = state.tabs.find((t) => t.id === id);
  if (!tab) return state;
  return patchTab(state, id, {
    status: "opening",
    sessionId: null,
    error: null,
    attempt: tab.attempt + 1,
  });
}

export function newTab(
  id: string,
  connectionId: Uuid,
  title: string,
  kind: ConnectionKind,
): Tab {
  return {
    id,
    connectionId,
    title,
    kind,
    status: "opening",
    sessionId: null,
    error: null,
    attempt: 0,
  };
}
