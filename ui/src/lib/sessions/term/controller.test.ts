import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { IpcError, type AppError } from "../../ipc";
import {
  ACK_BATCH_BYTES,
  MAX_WRITE_BYTES,
  TerminalController,
  type TermState,
} from "./controller";

const SID = "session-1";

function setup(overrides: Partial<Record<string, unknown>> = {}) {
  const written: Uint8Array[] = [];
  const doneCallbacks: Array<() => void> = [];
  const termWrite = vi.fn(async (_id: string, _data: Uint8Array) => undefined);
  const termAck = vi.fn(async (_id: string, _bytes: number) => undefined);
  const termResize = vi.fn(async (_id: string, _c: number, _r: number) => undefined);
  const sshHostKeyDecision = vi.fn(async (_id: string, _accept: boolean) => undefined);
  const states: TermState[] = [];
  const fatal: AppError[] = [];
  const problems: string[] = [];
  const controller = new TerminalController({
    ipc: { termWrite, termAck, termResize, sshHostKeyDecision, ...overrides } as never,
    sessionId: SID,
    terminal: {
      write: (data, done) => {
        written.push(data);
        doneCallbacks.push(done);
      },
    },
    onState: (s) => states.push(s),
    onFatal: (e) => fatal.push(e),
    onProblem: (m) => problems.push(m),
  });
  const data = (n: number) => ({ type: "data" as const, bytes: new Uint8Array(n).fill(65) });
  /** The terminal finishes parsing everything written so far. */
  const consumeAll = () => doneCallbacks.splice(0).forEach((cb) => cb());
  return { controller, written, termWrite, termAck, termResize, sshHostKeyDecision, states, fatal, problems, data, consumeAll };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("output and acknowledgements", () => {
  it("writes data to the terminal and only acknowledges what the terminal consumed", () => {
    const t = setup();
    t.controller.handleEvent(t.data(1000));
    expect(t.written).toHaveLength(1);
    expect(t.termAck).not.toHaveBeenCalled(); // not consumed yet
    vi.advanceTimersByTime(100);
    expect(t.termAck).not.toHaveBeenCalled(); // still not consumed: nothing to acknowledge
    t.consumeAll();
    expect(t.termAck).not.toHaveBeenCalled(); // batched: waits for the timer
    vi.advanceTimersByTime(8);
    expect(t.termAck).toHaveBeenCalledExactlyOnceWith(SID, 1000);
  });

  it("acknowledges immediately once a batch worth of bytes was consumed, and accumulates the rest", () => {
    const t = setup();
    const chunk = ACK_BATCH_BYTES / 2;
    t.controller.handleEvent(t.data(chunk));
    t.controller.handleEvent(t.data(chunk));
    t.controller.handleEvent(t.data(10));
    t.consumeAll();
    expect(t.termAck).toHaveBeenCalledTimes(1);
    expect(t.termAck).toHaveBeenLastCalledWith(SID, ACK_BATCH_BYTES);
    vi.advanceTimersByTime(8);
    expect(t.termAck).toHaveBeenCalledTimes(2);
    expect(t.termAck).toHaveBeenLastCalledWith(SID, 10);
  });

  it("the sum of the acknowledgements equals the bytes consumed", () => {
    const t = setup();
    let total = 0;
    for (let i = 1; i <= 50; i++) {
      t.controller.handleEvent(t.data(i * 997));
      total += i * 997;
    }
    t.consumeAll();
    vi.advanceTimersByTime(20);
    const acked = t.termAck.mock.calls.reduce((sum, [, n]) => sum + n, 0);
    expect(acked).toBe(total);
  });

  it("ignores framebuffer events", () => {
    const t = setup();
    t.controller.handleEvent({ type: "resize", w: 1, h: 1 });
    t.controller.handleEvent({ type: "closed", reason: "x" });
    expect(t.states).toHaveLength(0);
    expect(t.written).toHaveLength(0);
  });

  it("flushes the pending acknowledgement when the session ends", () => {
    const t = setup();
    t.controller.handleEvent(t.data(500));
    t.consumeAll();
    t.controller.handleEvent({ type: "exit", code: 0 });
    expect(t.termAck).toHaveBeenCalledExactlyOnceWith(SID, 500);
  });

  it("stops writing after dispose", () => {
    const t = setup();
    t.controller.dispose();
    t.controller.handleEvent(t.data(10));
    expect(t.written).toHaveLength(0);
  });
});

describe("state", () => {
  it("starts connecting and becomes running with the first data", () => {
    const t = setup();
    expect(t.controller.state.phase).toBe("connecting");
    t.controller.handleEvent(t.data(1));
    expect(t.controller.state.phase).toBe("running");
  });

  it("reports the exit code", () => {
    const t = setup();
    t.controller.handleEvent(t.data(1));
    t.controller.handleEvent({ type: "exit", code: 7 });
    expect(t.controller.state).toMatchObject({ phase: "exited", exitCode: 7 });
    t.controller.handleEvent({ type: "exit", code: null });
    expect(t.controller.state.exitCode).toBeNull();
  });

  it("a host key mismatch is shown by the view, not handed to the shell", () => {
    const t = setup();
    const error: AppError = { code: "host_key_mismatch", message: "HOST KEY CHANGED" };
    t.controller.handleEvent({ type: "error", error });
    expect(t.controller.state).toMatchObject({ phase: "mismatch", error });
    expect(t.fatal).toHaveLength(0);
  });

  it("any other error is fatal for the shell's error view", () => {
    const t = setup();
    const error: AppError = { code: "auth_failed", message: "no" };
    t.controller.handleEvent({ type: "error", error });
    expect(t.controller.state.phase).toBe("failed");
    expect(t.fatal).toEqual([error]);
  });
});

describe("host key prompt", () => {
  const prompt = {
    type: "hostKeyPrompt" as const,
    fingerprint: "SHA256:abc",
    algorithm: "ssh-ed25519",
    host: "jump.example",
    port: 2222,
  };

  it("shows the prompt and sends the user's decision", async () => {
    const t = setup();
    t.controller.handleEvent(prompt);
    expect(t.controller.state).toMatchObject({
      phase: "hostKeyPrompt",
      // The prompt says which hop the key belongs to (a jump host, here).
      prompt: { fingerprint: "SHA256:abc", algorithm: "ssh-ed25519", host: "jump.example", port: 2222 },
    });
    await t.controller.decideHostKey(true);
    expect(t.sshHostKeyDecision).toHaveBeenCalledExactlyOnceWith(SID, true);
    expect(t.controller.state).toMatchObject({ phase: "connecting", prompt: null });
  });

  it("sends a rejection as such", async () => {
    const t = setup();
    t.controller.handleEvent(prompt);
    await t.controller.decideHostKey(false);
    expect(t.sshHostKeyDecision).toHaveBeenCalledExactlyOnceWith(SID, false);
  });

  it("answers only while a prompt is pending", async () => {
    const t = setup();
    await t.controller.decideHostKey(true);
    expect(t.sshHostKeyDecision).not.toHaveBeenCalled();
    t.controller.handleEvent(prompt);
    await t.controller.decideHostKey(true);
    await t.controller.decideHostKey(true);
    expect(t.sshHostKeyDecision).toHaveBeenCalledTimes(1);
  });

  it("a failing decision is fatal", async () => {
    const t = setup({
      sshHostKeyDecision: vi.fn().mockRejectedValue(new IpcError("invalid_input", "no host key decision is pending")),
    });
    t.controller.handleEvent(prompt);
    await t.controller.decideHostKey(true);
    expect(t.fatal[0]?.code).toBe("invalid_input");
  });

  it("does not take input while the prompt is shown", async () => {
    const t = setup();
    t.controller.handleEvent(prompt);
    t.controller.input("x");
    await vi.runAllTimersAsync();
    expect(t.termWrite).not.toHaveBeenCalled();
  });
});

describe("input", () => {
  it("encodes typed text as UTF-8", async () => {
    const t = setup();
    t.controller.input("caffè ✓");
    await vi.runAllTimersAsync();
    expect(t.termWrite).toHaveBeenCalledTimes(1);
    const sent = t.termWrite.mock.calls[0]?.[1] ?? new Uint8Array();
    expect(new TextDecoder().decode(sent)).toBe("caffè ✓");
  });

  it("binary input maps one char to one byte", async () => {
    const t = setup();
    t.controller.inputBinary("\u0000ÿ\u0080");
    await vi.runAllTimersAsync();
    expect(Array.from(t.termWrite.mock.calls[0]?.[1] ?? [])).toEqual([0, 255, 128]);
  });

  it("sends one write at a time, in order, merging what queued meanwhile", async () => {
    let release: () => void = () => undefined;
    let inFlight = 0;
    let maxInFlight = 0;
    const order: string[] = [];
    const termWrite = vi.fn(async (_id: string, data: Uint8Array) => {
      inFlight++;
      maxInFlight = Math.max(maxInFlight, inFlight);
      order.push(new TextDecoder().decode(data));
      if (order.length === 1) await new Promise<void>((resolve) => (release = resolve));
      inFlight--;
    });
    const t = setup({ termWrite });
    t.controller.input("a");
    t.controller.input("b");
    t.controller.input("c");
    await vi.advanceTimersByTimeAsync(0);
    expect(termWrite).toHaveBeenCalledTimes(1); // "a" is in flight, "b" and "c" wait
    release();
    await vi.runAllTimersAsync();
    expect(order).toEqual(["a", "bc"]);
    expect(maxInFlight).toBe(1);
  });

  it("splits a huge paste into bounded writes that rebuild the original bytes", async () => {
    const t = setup();
    const paste = "x".repeat(MAX_WRITE_BYTES * 2 + 123);
    t.controller.input(paste);
    await vi.runAllTimersAsync();
    const sizes = t.termWrite.mock.calls.map(([, d]) => d.length);
    expect(sizes.every((n) => n <= MAX_WRITE_BYTES)).toBe(true);
    expect(sizes.reduce((a, b) => a + b, 0)).toBe(paste.length);
    expect(sizes.length).toBeGreaterThanOrEqual(3);
  });

  it("drops input after the session ended and does not complain", async () => {
    const t = setup();
    t.controller.handleEvent({ type: "exit", code: 0 });
    t.controller.input("late");
    await vi.runAllTimersAsync();
    expect(t.termWrite).not.toHaveBeenCalled();
    expect(t.problems).toHaveLength(0);
  });

  it("reports a failing write once, then keeps going", async () => {
    const termWrite = vi
      .fn()
      .mockRejectedValueOnce(new IpcError("io", "disk full"))
      .mockRejectedValueOnce(new IpcError("io", "disk full"))
      .mockResolvedValue(undefined);
    const t = setup({ termWrite });
    t.controller.input("a");
    await vi.runAllTimersAsync();
    t.controller.input("b");
    await vi.runAllTimersAsync();
    t.controller.input("c");
    await vi.runAllTimersAsync();
    expect(termWrite).toHaveBeenCalledTimes(3);
    expect(t.problems).toEqual(["disk full"]);
  });
});

describe("resize", () => {
  it("sends the size once per distinct value", () => {
    const t = setup();
    t.controller.resize(80, 24);
    t.controller.resize(80, 24);
    t.controller.resize(132, 50);
    expect(t.termResize.mock.calls).toEqual([
      [SID, 80, 24],
      [SID, 132, 50],
    ]);
  });

  it("ignores degenerate sizes", () => {
    const t = setup();
    t.controller.resize(0, 24);
    t.controller.resize(80, 0);
    expect(t.termResize).not.toHaveBeenCalled();
  });
});

describe("credential prompt", () => {
  const prompt = {
    type: "credentialPrompt" as const,
    kind: "sshKey" as const,
    usernameHint: "tester",
    domainHint: null,
    target: "127.0.0.1:22",
    error: null,
    canSaveToVault: true,
  };
  const answer = { username: "tester", password: "pw", keyPath: null, passphrase: null, domain: null };

  function withCredentials() {
    const sessionCredential = vi.fn(async (_id: string, _a: unknown, _save: boolean) => undefined);
    const sessionCredentialCancel = vi.fn(async (_id: string) => undefined);
    const saved = vi.fn();
    const states: TermState[] = [];
    const fatal: AppError[] = [];
    const controller = new TerminalController({
      ipc: {
        termWrite: vi.fn(async () => undefined),
        termAck: vi.fn(async () => undefined),
        termResize: vi.fn(async () => undefined),
        sshHostKeyDecision: vi.fn(async () => undefined),
        sessionCredential,
        sessionCredentialCancel,
      },
      sessionId: SID,
      terminal: { write: (_d, done) => done() },
      onState: (s) => states.push(s),
      onFatal: (e) => fatal.push(e),
      onProblem: () => undefined,
      onCredentialSaved: saved,
    });
    return { controller, sessionCredential, sessionCredentialCancel, saved, states, fatal };
  }

  it("enters the credentialPrompt phase with the prompt (without the event tag)", () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    expect(t.controller.state.phase).toBe("credentialPrompt");
    expect(t.controller.state.credential).toEqual({
      kind: "sshKey",
      usernameHint: "tester",
      domainHint: null,
      target: "127.0.0.1:22",
      error: null,
      canSaveToVault: true,
    });
  });

  it("sends the answer for this session, goes back to connecting and keeps nothing", async () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    await t.controller.submitCredential(answer, false);
    expect(t.sessionCredential).toHaveBeenCalledExactlyOnceWith(SID, answer, false);
    expect(t.controller.state).toMatchObject({ phase: "connecting", credential: null });
    expect(JSON.stringify(t.states)).not.toContain('"pw"');
    // A second submit without a new prompt does nothing.
    await t.controller.submitCredential(answer, false);
    expect(t.sessionCredential).toHaveBeenCalledTimes(1);
  });

  it("a new prompt after a wrong answer carries the error and keeps the session going", async () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    await t.controller.submitCredential(answer, false);
    t.controller.handleEvent({ ...prompt, error: "rejected" });
    expect(t.controller.state).toMatchObject({ phase: "credentialPrompt" });
    expect(t.controller.state.credential?.error).toBe("rejected");
    expect(t.fatal).toEqual([]);
  });

  it("cancelling tells the backend and shows the cancelled state, not an error", async () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    await t.controller.cancelCredential();
    expect(t.sessionCredentialCancel).toHaveBeenCalledExactlyOnceWith(SID);
    expect(t.controller.state).toMatchObject({ phase: "cancelled", credential: null });
    // The backend's cancelled error afterwards changes nothing and is not reported.
    t.controller.handleEvent({ type: "error", error: { code: "cancelled", message: "x" } });
    expect(t.controller.state.phase).toBe("cancelled");
    expect(t.fatal).toEqual([]);
  });

  it("a cancelled error on its own (no prompt shown) is a calm cancelled state", () => {
    const t = withCredentials();
    t.controller.handleEvent({ type: "error", error: { code: "cancelled", message: "x" } });
    expect(t.controller.state.phase).toBe("cancelled");
    expect(t.fatal).toEqual([]);
  });

  it("tells the shell to re-read the document once the saved credential got the session going", async () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    await t.controller.submitCredential(answer, true);
    expect(t.saved).not.toHaveBeenCalled();
    t.controller.handleEvent({ type: "data", bytes: new Uint8Array([1]) });
    t.controller.handleEvent({ type: "data", bytes: new Uint8Array([2]) });
    expect(t.saved).toHaveBeenCalledTimes(1);
  });

  it("does not report a save that was not asked for, or one whose attempt failed", async () => {
    const t = withCredentials();
    t.controller.handleEvent(prompt);
    await t.controller.submitCredential(answer, false);
    t.controller.handleEvent({ type: "data", bytes: new Uint8Array([1]) });
    expect(t.saved).not.toHaveBeenCalled();

    const u = withCredentials();
    u.controller.handleEvent(prompt);
    await u.controller.submitCredential(answer, true);
    u.controller.handleEvent({ ...prompt, error: "rejected" });
    await u.controller.cancelCredential();
    u.controller.handleEvent({ type: "data", bytes: new Uint8Array([1]) });
    expect(u.saved).not.toHaveBeenCalled();
  });

  it("a failing submit becomes a failure of the session", async () => {
    const t = withCredentials();
    t.sessionCredential.mockRejectedValueOnce(new IpcError("invalid_input", "no prompt pending"));
    t.controller.handleEvent(prompt);
    await t.controller.submitCredential(answer, false);
    expect(t.controller.state).toMatchObject({ phase: "failed", error: { code: "invalid_input" } });
    expect(t.fatal).toEqual([{ code: "invalid_input", message: "no prompt pending" }]);
  });

  it("does not type into a session that is waiting for credentials", async () => {
    const termWrite = vi.fn(async () => undefined);
    const controller = new TerminalController({
      ipc: {
        termWrite,
        termAck: vi.fn(async () => undefined),
        termResize: vi.fn(async () => undefined),
        sshHostKeyDecision: vi.fn(async () => undefined),
        sessionCredential: vi.fn(async () => undefined),
        sessionCredentialCancel: vi.fn(async () => undefined),
      },
      sessionId: SID,
      terminal: { write: () => undefined },
      onState: () => undefined,
      onFatal: () => undefined,
      onProblem: () => undefined,
    });
    controller.handleEvent(prompt);
    controller.input("hunter2");
    await vi.runAllTimersAsync();
    expect(termWrite).not.toHaveBeenCalled();
  });
});
