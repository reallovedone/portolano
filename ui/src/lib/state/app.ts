// The application's stores, created once per IPC implementation and shared through Svelte
// context (so tests can run the UI against their own mock).

import { getContext, setContext } from "svelte";

import { ipcMode, type Ipc } from "../ipc";
import { CloseConfirmStore } from "../tabs/closeConfirm.svelte";
import { TabsStore } from "../tabs/store.svelte";
import { SessionToolbarStore } from "../tabs/toolbar.svelte";
import { createMockFilePicker, tauriFilePicker, type FilePicker } from "../transfer/files";
import { DocumentStore } from "./document.svelte";
import { NoticeStore } from "./notices.svelte";
import { OverlayStore } from "./overlay.svelte";
import { UiStore } from "./ui.svelte";
import { UpdateStore, type UpdateTiming } from "./updates.svelte";
import { VaultStore } from "./vault.svelte";

export interface App {
  ipc: Ipc;
  /** Native file dialogs (canned paths in the mock). */
  files: FilePicker;
  doc: DocumentStore;
  vault: VaultStore;
  tabs: TabsStore;
  /** Confirmation before closing tabs with connected sessions (also used to quit). */
  closeConfirm: CloseConfirmStore;
  overlay: OverlayStore;
  notices: NoticeStore;
  ui: UiStore;
  /** What the session toolbar shows and whether it is visible. */
  toolbar: SessionToolbarStore;
  updates: UpdateStore;
}

export function createApp(
  ipc: Ipc,
  files: FilePicker = ipcMode === "mock" ? createMockFilePicker() : tauriFilePicker,
  updateTiming?: UpdateTiming,
): App {
  const doc = new DocumentStore(ipc);
  const vault = new VaultStore(ipc);
  const notices = new NoticeStore();
  const tabs = new TabsStore(ipc, doc, vault, notices);
  return {
    ipc,
    files,
    doc,
    vault,
    tabs,
    closeConfirm: new CloseConfirmStore(tabs),
    overlay: new OverlayStore(),
    notices,
    ui: new UiStore(),
    toolbar: new SessionToolbarStore(),
    updates: new UpdateStore(ipc, notices, updateTiming),
  };
}

const APP_KEY = Symbol("portolano.app");

export function provideApp(app: App): void {
  setContext(APP_KEY, app);
}

export function getApp(): App {
  const app = getContext<App | undefined>(APP_KEY);
  if (!app) throw new Error("getApp() called outside the application shell");
  return app;
}
