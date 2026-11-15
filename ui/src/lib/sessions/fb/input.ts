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
  requestFrame?: (callback: () => void) => number;
  cancelFrame?: (handle: number) => void;
  setTimer?: (callback: () => void, ms: number) => number;
  clearTimer?: (handle: number) => void;
}

/** The documented chord that releases the keyboard: Ctrl+Alt+Escape. */
export function isEscapeChord(e: Pick<KeyLike, "code" | "ctrlKey" | "altKey">): boolean {
  return e.code === "Escape" && e.ctrlKey && e.altKey;
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
    if (!isMappedKey(e.code)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "CapsLock") {
      // macOS reports one event per toggle (down when switched on, up when switched off).
      this.#capsDown = true;
      this.#tap(e.code);
      return;
    }
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
    if (!isMappedKey(e.code)) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "CapsLock") {
      if (this.#capsDown) this.#capsDown = false;
      else this.#tap(e.code);
      return;
    }
    if (e.code === "MetaLeft" || e.code === "MetaRight") {
      // Keys that were still down when Cmd was released will not get their own key-up.
      for (const code of [...this.#pressed]) if (!isModifier(code)) this.#release(code);
    }
    if (this.#pressed.has(e.code)) this.#release(e.code);
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
