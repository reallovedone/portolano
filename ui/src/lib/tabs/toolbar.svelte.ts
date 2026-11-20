// What the session toolbar shows for a tab, and whether it is shown at all.
//
// The toolbar knows nothing about RDP, VNC, SSH or web pages. Each session view publishes a
// `SessionToolbarModel` (plain data plus a few actions) through its session context; the toolbar
// reads it by tab id. A model reads reactive state lazily through getters, so the toolbar follows
// the view without being told about every change.

import { SvelteMap } from "svelte/reactivity";

import { loadJson, saveJson } from "../state/storage";

export const TOOLBAR_VISIBLE_KEY = "portolano.sessionToolbar.visible";

export interface ToolbarSecurity {
  /** The tooltip text of the lock. */
  readonly text: string;
  /** True when the connection is weaker than it should be: the lock is drawn in the warning colour. */
  readonly warning: boolean;
}

/** Fields of the web address bar. A reactive object, shared by the view and the toolbar. */
export interface WebAddressFields {
  address: string;
  error: string | null;
  loading: boolean;
  /** The address field has the focus: page changes must not overwrite what is being typed. */
  editing: boolean;
}

export interface WebToolbarControls {
  readonly fields: WebAddressFields;
  back(): void;
  forward(): void;
  reload(): void;
  /** Enter in the address field. */
  submit(): void;
}

export interface SessionToolbarModel {
  /** `host:port`, or the program of a local shell. */
  readonly target: string | null;
  /** Remote desktop size as `W×H`. */
  readonly resolution: string | null;
  readonly security: ToolbarSecurity | null;
  /** SSH only: null when the kind has no such state. */
  readonly connection: "connected" | "disconnected" | null;
  /** RDP and VNC. `enabled` is false until the session is connected. */
  readonly ctrlAltDel: { readonly enabled: boolean; send(): void } | null;
  readonly web: WebToolbarControls | null;
  /** Gives the keyboard back to the session view. */
  focus(): void;
}

export class SessionToolbarStore {
  /** Shown by default; the choice is kept in localStorage. */
  visible = $state(loadJson(TOOLBAR_VISIBLE_KEY, true, (v): v is boolean => typeof v === "boolean"));
  readonly #models = new SvelteMap<string, SessionToolbarModel>();

  setVisible(visible: boolean): void {
    this.visible = visible;
    saveJson(TOOLBAR_VISIBLE_KEY, visible);
  }

  toggle(): void {
    this.setVisible(!this.visible);
  }

  /** The model a tab's view published, or null (no view yet, or the session is not open). */
  model(tabId: string): SessionToolbarModel | null {
    return this.#models.get(tabId) ?? null;
  }

  /** Returns the function that withdraws `model` (ignored if another one replaced it). */
  publish(tabId: string, model: SessionToolbarModel): () => void {
    this.#models.set(tabId, model);
    return () => {
      if (this.#models.get(tabId) === model) this.#models.delete(tabId);
    };
  }
}
