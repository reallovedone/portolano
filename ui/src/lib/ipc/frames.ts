// Decoders for the binary Channel frames. Layouts are defined (and unit tested) in Rust:
// crates/core-model/src/fb.rs (FbEvent) and crates/session-term/src/lib.rs (TermEvent).
// Every frame is [tag u8][body], little endian.

import { IpcError } from "./errors";
import type {
  AppError,
  ClipboardEvent,
  ClipboardFailure,
  CredentialPrompt,
  FbEvent,
  TermEvent,
} from "./types";

const decoder = new TextDecoder();

function parseJson<T>(bytes: Uint8Array): T {
  try {
    return JSON.parse(decoder.decode(bytes)) as T;
  } catch (e) {
    throw new IpcError("internal", `malformed event frame: ${String(e)}`);
  }
}

const PROMPT_KINDS = ["password", "sshKey", "usernamePassword"];

/** Shared by both families (terminal tag 4, framebuffer tag 6). Missing optionals become null. */
function parseCredentialPrompt(body: Uint8Array): CredentialPrompt {
  const j = parseJson<Partial<CredentialPrompt>>(body);
  if (typeof j.kind !== "string" || !PROMPT_KINDS.includes(j.kind) || typeof j.target !== "string") {
    throw new IpcError("internal", "malformed credential prompt frame");
  }
  return {
    kind: j.kind,
    usernameHint: j.usernameHint ?? null,
    domainHint: j.domainHint ?? null,
    target: j.target,
    error: j.error ?? null,
    canSaveToVault: j.canSaveToVault === true,
  };
}

function u16s(view: DataView, offset: number, count: number): number[] {
  const out: number[] = [];
  for (let i = 0; i < count; i++) out.push(view.getUint16(offset + i * 2, true));
  return out;
}

function requireLength(frame: Uint8Array, min: number, what: string): void {
  if (frame.length < min) {
    throw new IpcError("internal", `truncated ${what} frame (${frame.length} bytes)`);
  }
}

export function decodeTermEvent(frame: Uint8Array): TermEvent {
  requireLength(frame, 1, "terminal");
  const body = frame.subarray(1);
  switch (frame[0]) {
    case 0:
      return { type: "data", bytes: body };
    case 1: {
      const j = parseJson<{ fingerprint: string; algorithm: string; host?: string; port?: number }>(
        body,
      );
      return {
        type: "hostKeyPrompt",
        fingerprint: j.fingerprint,
        algorithm: j.algorithm,
        host: typeof j.host === "string" ? j.host : "",
        port: typeof j.port === "number" ? j.port : 0,
      };
    }
    case 2:
      return { type: "exit", code: parseJson<{ code: number | null }>(body).code };
    case 3:
      return { type: "error", error: parseJson<AppError>(body) };
    case 4:
      return { type: "credentialPrompt", ...parseCredentialPrompt(body) };
    default:
      throw new IpcError("internal", `unknown terminal event tag ${frame[0]}`);
  }
}

const CLIPBOARD_FAILURES: ClipboardFailure[] = [
  "cancelled",
  "limit",
  "destination",
  "remote",
  "protocol",
  "nothing",
];

function num(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : 0;
}

function parseClipboardEvent(body: Uint8Array): ClipboardEvent {
  const j = parseJson<Record<string, unknown>>(body);
  switch (j.type) {
    case "remoteFiles":
      return { type: "remoteFiles", count: num(j.count), bytes: num(j.bytes), skipped: num(j.skipped) };
    case "progress":
      return {
        type: "progress",
        filesDone: num(j.filesDone),
        filesTotal: num(j.filesTotal),
        bytesDone: num(j.bytesDone),
        bytesTotal: num(j.bytesTotal),
      };
    case "done":
      return { type: "done", files: num(j.files), bytes: num(j.bytes), skipped: num(j.skipped) };
    case "failed":
      if (CLIPBOARD_FAILURES.includes(j.reason as ClipboardFailure)) {
        return { type: "failed", reason: j.reason as ClipboardFailure };
      }
      throw new IpcError("internal", "malformed clipboard frame");
    default:
      throw new IpcError("internal", "malformed clipboard frame");
  }
}

export function decodeFbEvent(frame: Uint8Array): FbEvent {
  requireLength(frame, 1, "framebuffer");
  const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
  const body = frame.subarray(1);
  switch (frame[0]) {
    case 0: {
      requireLength(frame, 5, "resize");
      const [w, h] = u16s(view, 1, 2);
      return { type: "resize", w, h };
    }
    case 1: {
      requireLength(frame, 9, "rect");
      const [x, y, w, h] = u16s(view, 1, 4);
      return { type: "rect", x, y, w, h, rgba: frame.subarray(9) };
    }
    case 2: {
      requireLength(frame, 9, "cursor");
      const [hotspotX, hotspotY, w, h] = u16s(view, 1, 4);
      return { type: "cursor", hotspotX, hotspotY, w, h, rgba: frame.subarray(9) };
    }
    case 3: {
      const j = parseJson<{ fingerprint: string; changed: boolean; stored?: string | null }>(body);
      return {
        type: "certPrompt",
        fingerprint: j.fingerprint,
        changed: j.changed,
        stored: typeof j.stored === "string" ? j.stored : null,
      };
    }
    case 4:
      return { type: "closed", reason: parseJson<{ reason: string }>(body).reason };
    case 5:
      return { type: "error", error: parseJson<AppError>(body) };
    case 6:
      return { type: "credentialPrompt", ...parseCredentialPrompt(body) };
    case 7:
      return { type: "security", encrypted: parseJson<{ encrypted: boolean }>(body).encrypted === true };
    case 8:
      return { type: "unencryptedPrompt" };
    case 9:
      return { type: "clipboard", event: parseClipboardEvent(body) };
    default:
      throw new IpcError("internal", `unknown framebuffer event tag ${frame[0]}`);
  }
}

/** Channel messages arrive as `ArrayBuffer` (raw body) or, defensively, a number array. */
export function toBytes(message: ArrayBuffer | ArrayBufferView | number[]): Uint8Array {
  if (message instanceof ArrayBuffer) return new Uint8Array(message);
  if (ArrayBuffer.isView(message)) {
    return new Uint8Array(message.buffer, message.byteOffset, message.byteLength);
  }
  return Uint8Array.from(message);
}
