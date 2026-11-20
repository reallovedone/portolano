import type { Uuid } from "../ipc";
import type { ConnectionKind } from "../tree/model";

/** Modal flows opened from several places (menus, shortcuts, buttons). One at a time. */
export type Dialog =
  // `kind` preselects the connection type when creating (welcome tiles); editing ignores it.
  | { type: "connection"; id: Uuid | null; parentId: Uuid | null; kind?: ConnectionKind }
  | { type: "folder"; id: Uuid | null; parentId: Uuid | null }
  | { type: "credentials" }
  // A file was chosen; the dialog previews it and imports it on confirmation.
  | { type: "import"; path: string }
  | { type: "createVault" }
  // "Check for updates…": version, the automatic-check setting, and the result of a check.
  | { type: "updates" }
  // Settings window (for now only the Appearance section).
  | { type: "settings" };

export class UiStore {
  dialog = $state<Dialog | null>(null);
  /** Incremented to ask the tree to focus its search box. */
  searchFocusRequest = $state(0);
  /** Selected node in the tree (also the default location for "new connection"). */
  selectedId = $state<Uuid | null>(null);

  open(dialog: Dialog): void {
    this.dialog = dialog;
  }

  closeDialog(): void {
    this.dialog = null;
  }

  focusSearch(): void {
    this.searchFocusRequest += 1;
  }
}
