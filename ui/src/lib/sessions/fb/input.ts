// Keyboard and pointer input of the framebuffer view.
//
// Everything goes through one ordered queue: Tauri runs async commands concurrently, so
// issuing `fbKey` calls without waiting could swap a key-down and its key-up. Consecutive
// pointer moves that have not been sent yet are collapsed (latest wins).

import { isMappedKey, isModifier } from "../keymap";
import { toRemote, wheelUnits, type RectLike, type Size } from "./geometry";

export interface InputApi {
  pointer(x: number, y: number, buttons: number, wheel: number): Promise<void>;
  key(code: string, down: boolean): Promise<void>;
}

export interface KeyLike {
  code: string;
  repeat: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  preventDefault(): void;
  stopPropagation(): void;
}

export interface PointerLike {
  clientX: number;
  clientY: number;
  buttons: number;
  preventDefault?(): void;
}

export interface WheelLike extends PointerLike {
  deltaY: number;
  deltaMode: number;
}

export interface Geometry {
  rect: RectLike;
  remote: Size;
}

export interface InputOptions {
  api: InputApi;
  /** Current canvas rectangle and remote size; null while there is no desktop yet. */
  geometry: () => Geometry | null;
  /** The escape chord was pressed: give the keyboard back to the shell. */
  onEscape?: () => void;
  onError?: (error: unknown) => void;
  /**
   * Whether the Mac shortcuts are on (macOS with the connection's option): Cmd+key goes to the
   * remote as Ctrl+key, and Cmd pressed and released alone as a tap of the Windows key.
   * Asked on every key event, so a change of the option applies at once.
   */
  macShortcuts?: () => boolean;
  requestFrame?: (callback: () => void) => number;
  cancelFrame?: (handle: number) => void;
  setTimer?: (callback: () => void, ms: number) => number;
  clearTimer?: (handle: number) => void;
}

/** The documented chord that releases the keyboard: Ctrl+Alt+Escape. */
export function isEscapeChord(e: Pick<KeyLike, "code" | "ctrlKey" | "altKey">): boolean {
  return e.code === "Escape" && e.ctrlKey && e.altKey;
}

/** Cmd+key combinations that stay with the app (Quit, Hide, Minimize, Close Tab). */
const APP_KEYS = new Set(["KeyQ", "KeyH", "KeyM", "KeyW"]);

/** Cmd+key combinations that stay with the app: Quit, Hide, Minimize, Close Tab, Reconnect (Cmd+Shift+R), Home (Cmd+Shift+H, already covered by Hide's key). */
function isAppCombo(e: KeyLike): boolean {
  return APP_KEYS.has(e.code) || (e.code === "KeyR" && e.shiftKey);
}

function isMeta(code: string): boolean {
  return code === "MetaLeft" || code === "MetaRight";
}

type Op =
  | { kind: "key"; code: string; down: boolean }
  | { kind: "pointer"; x: number; y: number; buttons: number; wheel: number };

const BUTTON_MASK = 0x1f;

export class InputController {
  readonly #pressed = new Set<string>();
  #buttons = 0;
  #last = { x: 0, y: 0 };
  #capsDown = false;
  /** Cmd keys held now, as far as this controller saw (Mac shortcuts only). */
  readonly #metaDown = new Set<string>();
  /** Another key was pressed while Cmd was held, so releasing Cmd is not a Windows key tap. */
  #metaUsed = false;
  #ops: Op[] = [];
  #draining = false;
  #moveFrame: number | null = null;
  #moveTimer: number | null = null;
  #latestMove: { x: number; y: number; buttons: number } | null = null;
  readonly #o: InputOptions;

  constructor(options: InputOptions) {
    this.#o = options;
  }

  /** Codes of the keys currently held down remotely. */
  get pressedKeys(): string[] {
    return [...this.#pressed];
  }

  // ---- keyboard ----

  keyDown(e: KeyLike): void {
    if (isEscapeChord(e)) {
      e.preventDefault();
      e.stopPropagation();
      this.releaseAll();
      this.#o.onEscape?.();
      return;
    }
    if (this.#macShortcuts() && e.metaKey && isAppCombo(e)) {
      // Left alone: the event goes on to the app. It also means Cmd is no lone tap.
      if (this.#metaDown.size > 0) this.#metaUsed = true;
      return;
    }
    if (!isMappedKey(e.code)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "CapsLock") {
      // macOS reports one event per toggle (down when switched on, up when switched off).
      this.#capsDown = true;
      this.#tap(e.code);
      return;
    }
    if (this.#macShortcuts() && this.#macKeyDown(e)) return;
    // Browsers repeat held modifiers; pressing them again would reset the remote state.
    if (e.repeat && isModifier(e.code)) return;
    this.#push({ kind: "key", code: e.code, down: true });
    this.#pressed.add(e.code);
    if (e.metaKey && !isModifier(e.code)) {
      // macOS does not deliver the key-up of a key pressed together with Cmd.
      this.#release(e.code);
    }
  }

  keyUp(e: KeyLike): void {
    if (this.#macShortcuts() && !isModifier(e.code) && this.#metaDown.size > 0) {
      // The key-up of a key pressed with Cmd: its key-down may never have arrived (the menu bar
      // takes some combinations), but Cmd was used all the same.
      this.#metaUsed = true;
      if (e.metaKey && isAppCombo(e)) return;
    }
    if (!isMappedKey(e.code)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "CapsLock") {
      if (this.#capsDown) this.#capsDown = false;
      else this.#tap(e.code);
      return;
    }
    if (this.#macShortcuts() && isMeta(e.code)) {
      // Cmd was never sent: released alone it is a tap of the Windows key (Start).
      if (this.#metaDown.delete(e.code) && this.#metaDown.size === 0) {
        if (!this.#metaUsed) this.#tap(e.code);
        this.#metaUsed = false;
      }
      return;
    }
    if (isMeta(e.code)) {
      // Keys that were still down when Cmd was released will not get their own key-up.
      for (const code of [...this.#pressed]) if (!isModifier(code)) this.#release(code);
    }
    if (this.#pressed.has(e.code)) this.#release(e.code);
  }

  #macShortcuts(): boolean {
    return this.#o.macShortcuts?.() === true;
  }

  /**
   * Mac shortcuts, key down. Returns whether the event was dealt with.
   *
   * - Cmd itself is not sent: it only arms the translation (see `keyUp` for the lone tap).
   * - Cmd+key becomes Ctrl down, key down, key up, Ctrl up, all queued together, so the remote
   *   never keeps a modifier and no later event can be slipped in between. The key-up is sent at
   *   once because macOS does not deliver one for a key pressed with Cmd. If a physical Ctrl is
   *   already down it is used as it is and left alone.
   * - Other modifiers (Shift, Alt) and keys without Cmd go the normal way.
   */
  #macKeyDown(e: KeyLike): boolean {
    if (isMeta(e.code)) {
      if (!e.repeat && !this.#metaDown.has(e.code)) {
        if (this.#metaDown.size === 0) this.#metaUsed = false;
        this.#metaDown.add(e.code);
      }
      return true;
    }
    if (this.#metaDown.size > 0) this.#metaUsed = true;
    if (isModifier(e.code) || !e.metaKey) return false;
    const ctrlHeld = this.#pressed.has("ControlLeft") || this.#pressed.has("ControlRight");
    if (!ctrlHeld) this.#push({ kind: "key", code: "ControlLeft", down: true });
    this.#tap(e.code);
    if (!ctrlHeld) this.#push({ kind: "key", code: "ControlLeft", down: false });
    return true;
  }

  #tap(code: string): void {
    this.#push({ kind: "key", code, down: true });
    this.#push({ kind: "key", code, down: false });
  }

  #release(code: string): void {
    this.#pressed.delete(code);
    this.#push({ kind: "key", code, down: false });
  }

  /** Releases every key and mouse button held remotely (blur, tab switch, escape chord). */
  releaseAll(): void {
    for (const code of [...this.#pressed]) this.#release(code);
    this.#capsDown = false;
    // Cmd was not sent in Mac shortcuts mode, so there is nothing to release and no tap to make.
    this.#metaDown.clear();
    this.#metaUsed = false;
    this.#cancelMove();
    if (this.#buttons !== 0) {
      this.#buttons = 0;
      this.#push({ kind: "pointer", x: this.#last.x, y: this.#last.y, buttons: 0, wheel: 0 });
    }
  }

  // ---- pointer ----

  #position(e: PointerLike): { x: number; y: number } | null {
    const g = this.#o.geometry();
    if (!g) return null;
    return toRemote(e.clientX, e.clientY, g.rect, g.remote);
  }

  pointerMove(e: PointerLike): void {
    const pos = this.#position(e);
    if (!pos) return;
    this.#last = pos;
    this.#buttons = e.buttons & BUTTON_MASK;
    this.#latestMove = { ...pos, buttons: this.#buttons };
    if (this.#moveFrame !== null) return;
    const request = this.#o.requestFrame ?? ((cb) => requestAnimationFrame(cb));
    const timer = this.#o.setTimer ?? ((cb, ms) => window.setTimeout(cb, ms));
    this.#moveFrame = request(() => this.#flushMove());
    // The frame never comes while the window is occluded; the timer covers that.
    this.#moveTimer = timer(() => this.#flushMove(), 50);
  }

  #flushMove(): void {
    const move = this.#latestMove;
    this.#cancelMove();
    if (move) this.#push({ kind: "pointer", ...move, wheel: 0 });
  }

  #cancelMove(): void {
    if (this.#moveFrame !== null) {
      (this.#o.cancelFrame ?? ((h) => cancelAnimationFrame(h)))(this.#moveFrame);
    }
    if (this.#moveTimer !== null) {
      (this.#o.clearTimer ?? ((h) => window.clearTimeout(h)))(this.#moveTimer);
    }
    this.#moveFrame = null;
    this.#moveTimer = null;
    this.#latestMove = null;
  }

  /** A button went down or up: sent at once, superseding any pending move. */
  pointerButton(e: PointerLike): void {
    const pos = this.#position(e);
    if (!pos) return;
    this.#cancelMove();
    this.#last = pos;
    this.#buttons = e.buttons & BUTTON_MASK;
    this.#push({ kind: "pointer", ...pos, buttons: this.#buttons, wheel: 0 });
  }

  wheel(e: WheelLike): void {
    const pos = this.#position(e);
    if (!pos) return;
    const units = wheelUnits(e.deltaY, e.deltaMode);
    if (units === 0) return;
    this.#cancelMove();
    this.#last = pos;
    this.#push({ kind: "pointer", ...pos, buttons: e.buttons & BUTTON_MASK, wheel: units });
  }

  dispose(): void {
    // Queued key releases still go out: a stuck modifier is worse than a late call.
    this.#cancelMove();
  }

  // ---- ordered queue ----

  #push(op: Op): void {
    const last = this.#ops[this.#ops.length - 1];
    if (
      op.kind === "pointer" &&
      last?.kind === "pointer" &&
      op.wheel === 0 &&
      last.wheel === 0 &&
      op.buttons === last.buttons
    ) {
      this.#ops[this.#ops.length - 1] = op;
    } else {
      this.#ops.push(op);
    }
    void this.#drain();
  }

  async #drain(): Promise<void> {
    if (this.#draining) return;
    this.#draining = true;
    try {
      let op = this.#ops.shift();
      while (op) {
        try {
          if (op.kind === "key") await this.#o.api.key(op.code, op.down);
          else await this.#o.api.pointer(op.x, op.y, op.buttons, op.wheel);
        } catch (e) {
          this.#o.onError?.(e);
        }
        op = this.#ops.shift();
      }
    } finally {
      this.#draining = false;
    }
  }
}
