// Terminal glue between the session bridge, the backend commands and the terminal widget.
// Framework-free so it can be unit tested with a fake bridge and a fake terminal.
//
// Flow control: the backend keeps at most 1 MiB of output sent and not
// yet acknowledged. The controller acknowledges bytes only after the terminal has parsed them
// (the `write` callback), in batches, so a slow renderer slows the backend down. Input goes
// out one `term_write` at a time, merged while queued, to keep the keystroke order.

import {
  toIpcError,
  type AppError,
  type CredentialAnswer,
  type CredentialPrompt,
  type Ipc,
  type SessionId,
} from "../../ipc";
import type { SessionEvent } from "../../tabs/session";

/** Acknowledge once this many bytes were consumed ... */
export const ACK_BATCH_BYTES = 128 * 1024;
/** ... or after this delay, whichever comes first. */
export const ACK_DELAY_MS = 8;
/** Largest single `term_write` body. */
export const MAX_WRITE_BYTES = 256 * 1024;

export type TermPhase =
  | "connecting"
  | "hostKeyPrompt"
  /** The backend waits for credentials typed by the user. */
  | "credentialPrompt"
  | "running"
  | "exited"
  | "mismatch"
  /** The user dismissed the credential prompt: not an error, offers "try again". */
  | "cancelled"
  | "failed";

export interface TermState {
  phase: TermPhase;
  /** Set in the `hostKeyPrompt` phase. */
  prompt: { fingerprint: string; algorithm: string; host: string; port: number } | null;
  /** Set in the `credentialPrompt` phase. Holds no secret. */
  credential: CredentialPrompt | null;
  /** Set in the `exited` phase (`null` when the remote reported no code). */
  exitCode: number | null;
  /** Set in the `mismatch` and `failed` phases. */
  error: AppError | null;
}

/** What the controller needs from the terminal widget. */
export interface TerminalPort {
  /** Parses `data`; `done` runs once the terminal has consumed it. */
  write(data: Uint8Array, done: () => void): void;
}

type TermIpc = Pick<
  Ipc,
  | "termWrite"
  | "termAck"
  | "termResize"
  | "sshHostKeyDecision"
  | "sessionCredential"
  | "sessionCredentialCancel"
>;

export interface ControllerOptions {
  ipc: TermIpc;
  sessionId: SessionId;
  terminal: TerminalPort;
  onState: (state: TermState) => void;
  /** A fatal error other than a host key mismatch: the shell shows its error view. */
  onFatal: (error: AppError) => void;
  /** A non-fatal problem worth telling the user (never contains secrets). */
  onProblem: (message: string) => void;
  /**
   * The session got going after the user asked to save the typed credential to the vault:
   * the backend has saved it by now, so the document and the vault list should be re-read.
   */
  onCredentialSaved?: () => void;
  ackBatchBytes?: number;
  ackDelayMs?: number;
}

const encoder = new TextEncoder();

/** Codes that only mean "the session is already gone": not worth a message. */
const BENIGN = new Set(["not_found", "unsupported"]);

export class TerminalController {
  readonly #o: ControllerOptions;
  #state: TermState = {
    phase: "connecting",
    prompt: null,
    credential: null,
    exitCode: null,
    error: null,
  };
  /** The last answer asked to be saved; cleared once the session proves it worked. */
  #saveRequested = false;
  #disposed = false;

  // acknowledgements
  #pendingAck = 0;
  #ackTimer: ReturnType<typeof setTimeout> | null = null;

  // input
  #queue: Uint8Array[] = [];
  #sending = false;
  #writeProblemShown = false;

  // resize
  #size: { cols: number; rows: number } | null = null;

  constructor(options: ControllerOptions) {
    this.#o = options;
  }

  get state(): TermState {
    return this.#state;
  }

  /** Bridge subscriber. Events of the framebuffer family are not for us and are ignored. */
  handleEvent(event: SessionEvent): void {
    if (this.#disposed) return;
    switch (event.type) {
      case "data":
        this.#onData(event.bytes);
        break;
      case "hostKeyPrompt":
        this.#set({
          phase: "hostKeyPrompt",
          prompt: {
            fingerprint: event.fingerprint,
            algorithm: event.algorithm,
            host: event.host,
            port: event.port,
          },
        });
        break;
      case "credentialPrompt": {
        const { type: _type, ...prompt } = event;
        this.#set({ phase: "credentialPrompt", credential: prompt, prompt: null });
        break;
      }
      case "exit":
        this.flushAcks();
        this.#queue = [];
        this.#set({ phase: "exited", exitCode: event.code, prompt: null, credential: null });
        break;
      case "error":
        this.flushAcks();
        this.#queue = [];
        if (event.error.code === "host_key_mismatch") {
          this.#set({ phase: "mismatch", error: event.error, prompt: null, credential: null });
        } else if (event.error.code === "cancelled") {
          // The user's own choice: a calm "cancelled" state, no error handling.
          this.#saveRequested = false;
          this.#set({ phase: "cancelled", error: null, prompt: null, credential: null });
        } else {
          this.#set({ phase: "failed", error: event.error, prompt: null, credential: null });
          this.#o.onFatal(event.error);
        }
        break;
      default:
        break;
    }
  }

  #set(patch: Partial<TermState>): void {
    this.#state = { ...this.#state, ...patch };
    this.#o.onState(this.#state);
  }

  #onData(bytes: Uint8Array): void {
    if (this.#state.phase === "connecting") {
      this.#set({ phase: "running" });
      if (this.#saveRequested) {
        this.#saveRequested = false;
        this.#o.onCredentialSaved?.();
      }
    }
    this.#o.terminal.write(bytes, () => this.#consumed(bytes.length));
  }

  // ---- flow control ----

  #consumed(n: number): void {
    if (this.#disposed) return;
    this.#pendingAck += n;
    if (this.#pendingAck >= (this.#o.ackBatchBytes ?? ACK_BATCH_BYTES)) {
      this.flushAcks();
    } else if (this.#ackTimer === null) {
      this.#ackTimer = setTimeout(() => this.flushAcks(), this.#o.ackDelayMs ?? ACK_DELAY_MS);
    }
  }

  /** Sends the pending acknowledgement now. */
  flushAcks(): void {
    if (this.#ackTimer !== null) {
      clearTimeout(this.#ackTimer);
      this.#ackTimer = null;
    }
    const bytes = this.#pendingAck;
    if (bytes === 0) return;
    this.#pendingAck = 0;
    this.#o.ipc.termAck(this.#o.sessionId, bytes).catch((e: unknown) => this.#report(e));
  }

  // ---- input ----

  get #acceptsInput(): boolean {
    return (
      !this.#disposed && (this.#state.phase === "connecting" || this.#state.phase === "running")
    );
  }

  /** Typed text (xterm `onData`). */
  input(text: string): void {
    this.#enqueue(encoder.encode(text));
  }

  /** Binary input (xterm `onBinary`: one char per byte, e.g. some mouse reports). */
  inputBinary(text: string): void {
    const bytes = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i) & 0xff;
    this.#enqueue(bytes);
  }

  #enqueue(bytes: Uint8Array): void {
    if (bytes.length === 0 || !this.#acceptsInput) return;
    this.#queue.push(bytes);
    void this.#pump();
  }

  /** At most one `term_write` in flight; whatever queued meanwhile goes out merged. */
  async #pump(): Promise<void> {
    if (this.#sending) return;
    this.#sending = true;
    try {
      while (this.#queue.length > 0 && !this.#disposed) {
        const chunk = this.#takeChunk();
        try {
          await this.#o.ipc.termWrite(this.#o.sessionId, chunk);
          this.#writeProblemShown = false;
        } catch (e) {
          const error = toIpcError(e);
          // After the end of the session (or a close) the backend answers `io`/`not_found`.
          if (this.#acceptsInput && !BENIGN.has(error.code) && !this.#writeProblemShown) {
            this.#writeProblemShown = true;
            this.#o.onProblem(error.message);
          }
        }
      }
    } finally {
      this.#sending = false;
    }
  }

  #takeChunk(): Uint8Array {
    const first = this.#queue.shift();
    if (first === undefined) return new Uint8Array(0);
    if (first.length >= MAX_WRITE_BYTES) {
      // A huge paste: send the head, keep the rest at the front of the queue.
      if (first.length > MAX_WRITE_BYTES) this.#queue.unshift(first.subarray(MAX_WRITE_BYTES));
      return first.subarray(0, MAX_WRITE_BYTES);
    }
    const parts = [first];
    let total = first.length;
    while (this.#queue.length > 0) {
      const next = this.#queue[0];
      if (next === undefined || total + next.length > MAX_WRITE_BYTES) break;
      parts.push(next);
      total += next.length;
      this.#queue.shift();
    }
    if (parts.length === 1) return first;
    const merged = new Uint8Array(total);
    let offset = 0;
    for (const part of parts) {
      merged.set(part, offset);
      offset += part.length;
    }
    return merged;
  }

  // ---- host key ----

  /** The user's answer to the host key prompt. */
  async decideHostKey(accept: boolean): Promise<void> {
    if (this.#state.phase !== "hostKeyPrompt") return;
    this.#set({ phase: "connecting", prompt: null });
    try {
      await this.#o.ipc.sshHostKeyDecision(this.#o.sessionId, accept);
    } catch (e) {
      const error = toIpcError(e);
      const appError = { code: error.code, message: error.message };
      this.#set({ phase: "failed", error: appError });
      this.#o.onFatal(appError);
    }
  }

  // ---- credentials ----

  /**
   * The user's answer to the credential prompt. The secrets are handed to the backend and
   * dropped here: nothing of the answer is kept.
   */
  async submitCredential(answer: CredentialAnswer, saveToVault: boolean): Promise<void> {
    if (this.#state.phase !== "credentialPrompt") return;
    this.#saveRequested = saveToVault;
    this.#set({ phase: "connecting", credential: null });
    try {
      await this.#o.ipc.sessionCredential(this.#o.sessionId, answer, saveToVault);
    } catch (e) {
      // The message of an IPC error never carries the secret.
      const error = toIpcError(e);
      const appError = { code: error.code, message: error.message };
      this.#set({ phase: "failed", error: appError });
      this.#o.onFatal(appError);
    }
  }

  /** The user dismissed the credential prompt: the session ends as cancelled. */
  async cancelCredential(): Promise<void> {
    if (this.#state.phase !== "credentialPrompt") return;
    this.#saveRequested = false;
    this.#set({ phase: "cancelled", credential: null, error: null });
    try {
      await this.#o.ipc.sessionCredentialCancel(this.#o.sessionId);
    } catch (e) {
      this.#report(e);
    }
  }

  // ---- size ----

  /** Reports a new size to the backend (same size twice is ignored). */
  resize(cols: number, rows: number): void {
    if (this.#disposed || cols < 1 || rows < 1) return;
    if (this.#size?.cols === cols && this.#size.rows === rows) return;
    this.#size = { cols, rows };
    this.#o.ipc
      .termResize(this.#o.sessionId, cols, rows)
      .catch((e: unknown) => this.#report(e));
  }

  #report(e: unknown): void {
    const error = toIpcError(e);
    if (!BENIGN.has(error.code) && !this.#disposed) this.#o.onProblem(error.message);
  }

  dispose(): void {
    if (this.#disposed) return;
    this.flushAcks();
    this.#disposed = true;
    this.#queue = [];
  }
}
