// Tells the backend when a graphical session holds the keyboard with the Mac shortcuts on, so the
// native Edit menu can leave the macOS menu bar (its ⌘C, ⌘V... would otherwise take the keys
// before the remote sees them).
//
// Several session views can exist at once (one per tab) and their changes are not ordered: the
// menu is out of the menu bar while at least one view holds the keyboard, and only a change of
// that state is sent.

import type { Ipc } from "../../ipc";

export class SessionKeysMenu {
  readonly #holders = new Set<object>();
  #sent = false;
  readonly #ipc: Pick<Ipc, "menuSetSessionKeys" | "uiLog">;

  constructor(ipc: Pick<Ipc, "menuSetSessionKeys" | "uiLog">) {
    this.#ipc = ipc;
  }

  /**
   * `holder` (a session view) holds the keyboard now, or no longer does. With `force` a held
   * keyboard is sent again even if the backend was told already: the backend puts the menu back
   * by itself when the window loses the keyboard, and this call may be all that follows.
   */
  set(holder: object, captured: boolean, force = false): void {
    if (captured) this.#holders.add(holder);
    else this.#holders.delete(holder);
    const want = this.#holders.size > 0;
    if (want === this.#sent && !(force && want)) return;
    this.#sent = want;
    this.#ipc.menuSetSessionKeys(want).catch((e: unknown) => {
      // Not sent after all: the next change (or the next call) must try again.
      this.#sent = !want;
      // Safe to drop: the menu only keeps or loses its shortcuts; the failure is in the backend log.
      this.#ipc
        .uiLog("warn", `menu keys failed: ${e instanceof Error ? e.message : String(e)}`)
        .catch(() => undefined);
    });
  }
}

const instances = new WeakMap<object, SessionKeysMenu>();

/** The shared tracker of an `Ipc`. */
export function sessionKeysMenu(ipc: Ipc): SessionKeysMenu {
  let menu = instances.get(ipc);
  if (!menu) {
    menu = new SessionKeysMenu(ipc);
    instances.set(ipc, menu);
  }
  return menu;
}
