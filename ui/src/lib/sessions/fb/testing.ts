// Test helpers: deterministic timers and animation frames, and a recording `FbApi`.

import type { FbApi } from "./controller";

export class ManualClock {
  #now = 0;
  #next = 1;
  readonly #timers = new Map<number, { at: number; cb: () => void }>();
  readonly #frames = new Map<number, () => void>();

  readonly setTimer = (cb: () => void, ms: number): number => {
    const id = this.#next++;
    this.#timers.set(id, { at: this.#now + ms, cb });
    return id;
  };
  readonly clearTimer = (id: number): void => {
    this.#timers.delete(id);
  };
  readonly requestFrame = (cb: () => void): number => {
    const id = this.#next++;
    this.#frames.set(id, cb);
    return id;
  };
  readonly cancelFrame = (id: number): void => {
    this.#frames.delete(id);
  };

  get pendingTimers(): number {
    return this.#timers.size;
  }
  get pendingFrames(): number {
    return this.#frames.size;
  }

  /** Fires every pending animation frame callback. */
  frame(): void {
    const due = [...this.#frames.values()];
    this.#frames.clear();
    for (const cb of due) cb();
  }

  /** Moves time forward, firing the timers that become due. */
  advance(ms: number): void {
    this.#now += ms;
    for (const [id, t] of [...this.#timers]) {
      if (t.at <= this.#now && this.#timers.delete(id)) t.cb();
    }
  }
}

export type Call =
  | { fn: "pointer"; x: number; y: number; buttons: number; wheel: number }
  | { fn: "key"; code: string; down: boolean }
  | { fn: "resize"; width: number; height: number }
  | { fn: "setVisible"; visible: boolean }
  | { fn: "certDecision"; accept: boolean }
  | { fn: "unencryptedDecision"; allow: boolean }
  | { fn: "forgetCertificate" }
  | { fn: "credential"; username: string; saveToVault: boolean }
  | { fn: "credentialCancel" };

export function recordingApi(options: { delayMs?: number } = {}): { api: FbApi; calls: Call[] } {
  const calls: Call[] = [];
  const done = async (): Promise<void> => {
    if (options.delayMs) await new Promise((r) => setTimeout(r, options.delayMs));
  };
  const api: FbApi = {
    async pointer(x, y, buttons, wheel) {
      await done();
      calls.push({ fn: "pointer", x, y, buttons, wheel });
    },
    async key(code, down) {
      await done();
      calls.push({ fn: "key", code, down });
    },
    async resize(width, height) {
      calls.push({ fn: "resize", width, height });
    },
    async setVisible(visible) {
      calls.push({ fn: "setVisible", visible });
    },
    async certDecision(accept) {
      calls.push({ fn: "certDecision", accept });
    },
    async unencryptedDecision(allow) {
      calls.push({ fn: "unencryptedDecision", allow });
    },
    async forgetCertificate() {
      calls.push({ fn: "forgetCertificate" });
    },
    async credential(answer, saveToVault) {
      // The secret itself is deliberately not recorded.
      calls.push({ fn: "credential", username: answer.username, saveToVault });
    },
    async credentialCancel() {
      calls.push({ fn: "credentialCancel" });
    },
  };
  return { api, calls };
}

export function fakeKey(
  code: string,
  extra: Partial<{ repeat: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean }> = {},
) {
  const e = {
    code,
    repeat: false,
    ctrlKey: false,
    altKey: false,
    metaKey: false,
    prevented: false,
    stopped: false,
    ...extra,
    preventDefault() {
      e.prevented = true;
    },
    stopPropagation() {
      e.stopped = true;
    },
  };
  return e;
}

/** Let queued promise callbacks run. */
export async function flushPromises(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve();
}
