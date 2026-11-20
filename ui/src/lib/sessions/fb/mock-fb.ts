// A fake framebuffer backend for the mock IPC: lets `FramebufferView` run without a server.
//
// It shows a test pattern, echoes the pointer as a small square and every key as a coloured
// cell, asks for the credentials when the connection has none, asks for certificate trust once
// per host (RDP, and VNC over TLS; plain VNC has no certificate), honours
// `fbResize`/`fbSetVisible`, and sends a cursor shape. Nothing here is secure or realistic; it
// only speaks `FbEvent`.
//
// Order of the prompts, like the real backends: RDP asks for credentials before the
// certificate; VNC over TLS (VeNCrypt) shows the certificate first and asks for user name and
// password after it. Plain VNC reports `security {encrypted: false}`, VNC over TLS
// `{encrypted: true}`, both right before the first `resize`.
//
// Plain VNC asks `unencryptedPrompt` once per host before it connects (the password is only
// "sent" after the answer). A host whose name contains "changed" has a stored certificate that
// differs from the one it presents: RDP and VNC over TLS end with `certPrompt {changed: true}`
// and nothing else, until `fbForgetCertificate` removes the stored one.
//
// Which mock VNC connection simulates a TLS server: one tagged `tls` or with port 15901 (the
// port of the `vnc-tls` test container). Every other VNC connection stays plain.

import { IpcError } from "../../ipc/errors";
import { MockPrompter, NO_PROMPTS, type MockPromptHooks } from "../../ipc/mock-credentials";
import type {
  Connection,
  CredentialAnswer,
  CredentialPromptKind,
  Document,
  FbEvent,
  SessionId,
  Uuid,
} from "../../ipc/types";

const FAKE_FINGERPRINT =
  "AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99";
const STALE_FINGERPRINT =
  "00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF:00:11:22:33:44:55:66:77:88:99:AA:BB:CC:DD:EE:FF";
const CELL = 14;
const POINTER_SIZE = 9;

/** Colour bars on top, a grid below; RGBA, opaque. */
export function testPattern(w: number, h: number): Uint8Array {
  const rgba = new Uint8Array(w * h * 4);
  const bars = [
    [255, 255, 255],
    [255, 255, 0],
    [0, 255, 255],
    [0, 255, 0],
    [255, 0, 255],
    [255, 0, 0],
    [0, 0, 255],
    [0, 0, 0],
  ];
  const barHeight = Math.max(1, Math.floor(h / 3));
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      let c: number[];
      if (y < barHeight) {
        c = bars[Math.min(bars.length - 1, Math.floor((x * bars.length) / w))] ?? [0, 0, 0];
      } else {
        const grid = x % 32 === 0 || y % 32 === 0;
        c = grid ? [90, 100, 120] : [24 + ((x * 40) / w) | 0, 28, 40 + ((y * 60) / h) | 0];
      }
      rgba[i] = c[0] ?? 0;
      rgba[i + 1] = c[1] ?? 0;
      rgba[i + 2] = c[2] ?? 0;
      rgba[i + 3] = 255;
    }
  }
  return rgba;
}

interface Session {
  id: SessionId;
  host: string;
  w: number;
  h: number;
  pattern: Uint8Array;
  image: Uint8Array;
  visible: boolean;
  connected: boolean;
  closed: boolean;
  /** RDP and VNC over TLS ask for certificate trust; plain VNC has no certificate. */
  certificate: boolean;
  /** The certificate is shown before the credentials are asked (VNC over TLS). */
  certFirst: boolean;
  /** The `security` event this session reports once connected (`null`: none). */
  security: Extract<FbEvent, { type: "security" }> | null;
  /** Plain VNC: the user must agree to an unencrypted password exchange (once per host). */
  plain: boolean;
  /** The session ended with the changed-certificate report: nothing to do but reconnect. */
  ended: boolean;
  keysDrawn: number;
  /** An RDP connection that shares its clipboard: the mock offers two remote files. */
  clipboard: boolean;
  /** Timer of a running mock save. */
  saveTimer: ReturnType<typeof setInterval> | null;
  pointer: { x: number; y: number } | null;
  prompter: MockPrompter | null;
  onEvent: (event: FbEvent) => void;
}

function hash(code: string): number {
  let h = 2166136261;
  for (const ch of code) h = Math.imul(h ^ ch.charCodeAt(0), 16777619);
  return h >>> 0;
}

function crosshair(): FbEvent {
  const size = 16;
  const rgba = new Uint8Array(size * size * 4);
  for (let i = 0; i < size; i++) {
    for (const [x, y] of [
      [i, size / 2],
      [size / 2, i],
    ] as const) {
      const o = (y * size + x) * 4;
      rgba[o] = 255;
      rgba[o + 1] = 40;
      rgba[o + 2] = 40;
      rgba[o + 3] = 255;
    }
  }
  return { type: "cursor", hotspotX: size / 2, hotspotY: size / 2, w: size, h: size, rgba };
}

/** Size of the two files the mock offers on its remote clipboard. */
const MOCK_CLIPBOARD_BYTES = 2_500_000;

export type MockTransport = "rdp" | "vnc" | "vnc-tls";

/** Whether a mock VNC connection simulates a server with VeNCrypt (see the header). */
export function isMockVncTls(connection: Connection): boolean {
  return (
    connection.options.type === "vnc" &&
    (connection.port === 15901 || connection.tags.some((tag) => tag.toLowerCase() === "tls"))
  );
}

export class MockFramebuffer {
  readonly #sessions = new Map<SessionId, Session>();
  readonly #trusted = new Set<string>();
  /** Hosts for which the user agreed to an unencrypted VNC password exchange. */
  readonly #plainAllowed = new Set<string>();
  /** Hosts whose stored (stale) certificate was removed. */
  readonly #forgotten = new Set<string>();
  #counter = 0;

  /**
   * `host` selects the certificate prompt (asked once per host, except for plain VNC). With
   * `ask`, the session asks the user for a credential of that `kind` (before the certificate,
   * except for VNC over TLS).
   */
  open(
    host: string,
    width: number,
    height: number,
    onEvent: (event: FbEvent) => void,
    ask: { connection: Connection; kind: CredentialPromptKind; hooks: MockPromptHooks } | null = null,
    transport: MockTransport = "rdp",
    clipboard = false,
  ): SessionId {
    const id: SessionId = `mock-fb-${++this.#counter}`;
    const w = Math.max(200, width);
    const h = Math.max(200, height);
    const pattern = testPattern(w, h);
    const session: Session = {
      id,
      host,
      w,
      h,
      pattern,
      image: pattern.slice(),
      visible: true,
      connected: false,
      closed: false,
      certificate: transport !== "vnc",
      certFirst: transport === "vnc-tls",
      security:
        transport === "rdp"
          ? { type: "security", encrypted: true, auth: "negotiate" }
          : { type: "security", encrypted: transport === "vnc-tls" },
      plain: transport === "vnc",
      ended: false,
      keysDrawn: 0,
      clipboard,
      saveTimer: null,
      pointer: null,
      prompter: null,
      onEvent,
    };
    if (ask) {
      session.prompter = new MockPrompter(ask.connection, ask.kind, ask.hooks, {
        emit: (prompt) => onEvent({ type: "credentialPrompt", ...prompt }),
        granted: () => this.#afterCredentials(session),
        failed: (code, message) => {
          session.closed = true;
          onEvent({ type: "error", error: { code, message } });
        },
      });
    }
    this.#sessions.set(id, session);
    // Events arrive after `open` resolved, like a real connection.
    setTimeout(() => {
      if (session.closed) return;
      if (this.#certificateChanged(session)) return;
      if (session.certFirst && !this.#trusted.has(session.host)) {
        session.onEvent({ type: "certPrompt", fingerprint: FAKE_FINGERPRINT, changed: false, stored: null });
      } else this.#askOrConnect(session);
    }, 0);
    return id;
  }

  /** A "changed" host presents a certificate that differs from the stored one: the attempt
   *  ends with the report (like the real backends); there is nothing to decide. */
  #certificateChanged(s: Session): boolean {
    if (!s.certificate || !s.host.toLowerCase().includes("changed")) return false;
    if (this.#forgotten.has(s.host)) return false;
    s.ended = true;
    s.onEvent({
      type: "certPrompt",
      fingerprint: FAKE_FINGERPRINT,
      changed: true,
      stored: STALE_FINGERPRINT,
    });
    return true;
  }

  #askOrConnect(s: Session): void {
    if (s.prompter) s.prompter.ask();
    else this.#afterCredentials(s);
  }

  /** Credentials settled (or not needed): the certificate (unless it came first), then the
   *  desktop. */
  #afterCredentials(s: Session): void {
    if (s.closed) return;
    if (!s.certificate && s.plain && !this.#plainAllowed.has(s.host)) {
      s.onEvent({ type: "unencryptedPrompt" });
      return;
    }
    if (!s.certificate || s.certFirst || this.#trusted.has(s.host)) {
      this.#connect(s);
    } else if (!this.#certificateChanged(s)) {
      s.onEvent({ type: "certPrompt", fingerprint: FAKE_FINGERPRINT, changed: false, stored: null });
    }
  }

  credential(id: SessionId, answer: CredentialAnswer, saveToVault: boolean): void {
    const s = this.#get(id);
    if (!s.prompter) throw new IpcError("invalid_input", "no credential prompt is pending");
    s.prompter.answer(answer, saveToVault);
  }

  credentialCancel(id: SessionId): void {
    const s = this.#get(id);
    if (!s.prompter) throw new IpcError("invalid_input", "no credential prompt is pending");
    s.prompter.cancel();
  }

  #get(id: SessionId): Session {
    const session = this.#sessions.get(id);
    if (!session || session.closed) throw new IpcError("not_found", `session ${id} not found`);
    return session;
  }

  /** Whether `id` is a session of this fake (so `sessionClose` knows who owns it). */
  has(id: SessionId): boolean {
    return this.#sessions.has(id);
  }

  #connect(s: Session): void {
    s.connected = true;
    if (s.security !== null) s.onEvent(s.security);
    s.onEvent({ type: "resize", w: s.w, h: s.h });
    this.#sendRect(s, 0, 0, s.w, s.h);
    s.onEvent(crosshair());
    if (s.clipboard) {
      s.onEvent({
        type: "clipboard",
        event: { type: "remoteFiles", count: 2, bytes: MOCK_CLIPBOARD_BYTES, skipped: 0 },
      });
    }
  }

  /** Pretends to save the remote clipboard files: a few progress steps, then done. */
  clipboardSave(id: SessionId): boolean {
    const s = this.#get(id);
    if (!s.connected || !s.clipboard) {
      throw new IpcError("invalid_input", "clipboard sharing is off for this connection");
    }
    if (s.saveTimer !== null) return true;
    let step = 0;
    s.saveTimer = setInterval(() => {
      step += 1;
      if (step >= 4) {
        if (s.saveTimer !== null) clearInterval(s.saveTimer);
        s.saveTimer = null;
        s.onEvent({ type: "clipboard", event: { type: "done", files: 2, bytes: MOCK_CLIPBOARD_BYTES, skipped: 0 } });
        return;
      }
      s.onEvent({
        type: "clipboard",
        event: {
          type: "progress",
          filesDone: step > 2 ? 1 : 0,
          filesTotal: 2,
          bytesDone: Math.floor((MOCK_CLIPBOARD_BYTES * step) / 4),
          bytesTotal: MOCK_CLIPBOARD_BYTES,
        },
      });
    }, 400);
    return true;
  }

  clipboardCancel(id: SessionId): void {
    const s = this.#get(id);
    if (s.saveTimer === null) return;
    clearInterval(s.saveTimer);
    s.saveTimer = null;
    s.onEvent({ type: "clipboard", event: { type: "failed", reason: "cancelled" } });
  }

  #sendRect(s: Session, x: number, y: number, w: number, h: number): void {
    if (!s.visible || !s.connected) return;
    const x0 = Math.max(0, x);
    const y0 = Math.max(0, y);
    const w0 = Math.min(s.w - x0, w - (x0 - x));
    const h0 = Math.min(s.h - y0, h - (y0 - y));
    if (w0 <= 0 || h0 <= 0) return;
    const rgba = new Uint8Array(w0 * h0 * 4);
    for (let row = 0; row < h0; row++) {
      const from = ((y0 + row) * s.w + x0) * 4;
      rgba.set(s.image.subarray(from, from + w0 * 4), row * w0 * 4);
    }
    s.onEvent({ type: "rect", x: x0, y: y0, w: w0, h: h0, rgba });
  }

  #fill(s: Session, x: number, y: number, w: number, h: number, rgb: [number, number, number]): void {
    for (let row = y; row < Math.min(s.h, y + h); row++) {
      for (let col = x; col < Math.min(s.w, x + w); col++) {
        if (row < 0 || col < 0) continue;
        const o = (row * s.w + col) * 4;
        s.image[o] = rgb[0];
        s.image[o + 1] = rgb[1];
        s.image[o + 2] = rgb[2];
        s.image[o + 3] = 255;
      }
    }
  }

  pointer(id: SessionId, x: number, y: number, buttons: number, wheel: number): void {
    const s = this.#get(id);
    if (!s.connected) return;
    const pattern = s.pattern;
    if (s.pointer) {
      // Restore the pattern under the previous square.
      const p = s.pointer;
      for (let row = p.y; row < Math.min(s.h, p.y + POINTER_SIZE); row++) {
        const from = (row * s.w + p.x) * 4;
        const n = Math.min(POINTER_SIZE, s.w - p.x) * 4;
        s.image.set(pattern.subarray(from, from + n), from);
      }
      this.#sendRect(s, p.x, p.y, POINTER_SIZE, POINTER_SIZE);
    }
    s.pointer = { x, y };
    const color: [number, number, number] =
      wheel !== 0 ? [255, 160, 0] : buttons & 1 ? [255, 60, 60] : buttons & 2 ? [60, 160, 255] : [255, 255, 255];
    this.#fill(s, x, y, POINTER_SIZE, POINTER_SIZE, color);
    this.#sendRect(s, x, y, POINTER_SIZE, POINTER_SIZE);
  }

  key(id: SessionId, code: string, down: boolean): void {
    const s = this.#get(id);
    if (!s.connected || !down) return;
    const perRow = Math.max(1, Math.floor(s.w / CELL));
    const index = s.keysDrawn++;
    const x = (index % perRow) * CELL;
    const y = s.h - CELL * (1 + Math.floor(index / perRow) % 4);
    const hue = hash(code);
    this.#fill(s, x, y, CELL - 1, CELL - 1, [(hue & 255) | 64, ((hue >> 8) & 255) | 64, ((hue >> 16) & 255) | 64]);
    this.#sendRect(s, x, y, CELL, CELL);
  }

  resize(id: SessionId, width: number, height: number): void {
    const s = this.#get(id);
    if (!s.connected) return;
    s.w = Math.max(200, width - (width % 2));
    s.h = Math.max(200, height);
    s.pattern = testPattern(s.w, s.h);
    s.image = s.pattern.slice();
    s.pointer = null;
    s.keysDrawn = 0;
    s.onEvent({ type: "resize", w: s.w, h: s.h });
    this.#sendRect(s, 0, 0, s.w, s.h);
  }

  setVisible(id: SessionId, visible: boolean): void {
    const s = this.#get(id);
    const was = s.visible;
    s.visible = visible;
    if (visible && !was) this.#sendRect(s, 0, 0, s.w, s.h);
  }

  unencryptedDecision(id: SessionId, allow: boolean): void {
    const s = this.#get(id);
    if (s.connected || !s.plain) return;
    if (allow) {
      this.#plainAllowed.add(s.host);
      this.#connect(s);
    } else {
      s.closed = true;
      s.onEvent({ type: "closed", reason: "the connection is not encrypted and you did not allow that" });
    }
  }

  /** Removes the stored (stale) certificate of the session's host. */
  forgetCertificate(id: SessionId): void {
    const s = this.#get(id);
    this.#forgotten.add(s.host);
  }

  certDecision(id: SessionId, accept: boolean): void {
    const s = this.#get(id);
    if (s.connected || s.ended) return;
    if (accept) {
      this.#trusted.add(s.host);
      // VNC over TLS asks for the credentials only now; the others are done by this point.
      if (s.certFirst) this.#askOrConnect(s);
      else this.#connect(s);
    } else {
      s.closed = true;
      s.onEvent({ type: "closed", reason: "the server certificate was not accepted" });
    }
  }

  close(id: SessionId): boolean {
    const s = this.#sessions.get(id);
    if (!s) return false;
    s.closed = true;
    if (s.saveTimer !== null) clearInterval(s.saveTimer);
    this.#sessions.delete(id);
    return true;
  }
}

/** The `Ipc` side of the fake: validation and the `unsupported` default (see `MockOptions`). */
export function createMockFramebuffer(
  getDoc: () => Document,
  enabled: boolean,
  hooks: MockPromptHooks = NO_PROMPTS,
) {
  const fake = new MockFramebuffer();
  const off = (what: string): Promise<never> =>
    Promise.reject(new IpcError("unsupported", `${what} is not available in the mock IPC`));
  const run = <T>(what: string, f: () => T): Promise<T> => {
    if (!enabled) return off(what);
    try {
      return Promise.resolve(f());
    } catch (e) {
      return Promise.reject(e);
    }
  };
  return {
    open: (
      connectionId: Uuid,
      width: number,
      height: number,
      onEvent: (event: FbEvent) => void,
      askCredentials = false,
    ): Promise<SessionId> =>
      run("fbOpen", () => {
        const node = getDoc().nodes.find((n) => n.id === connectionId);
        if (!node) throw new IpcError("not_found", `connection ${connectionId} not found`);
        if (node.type !== "connection") throw new IpcError("invalid_input", "not a connection");
        if (node.options.type !== "rdp" && node.options.type !== "vnc") {
          throw new IpcError("invalid_input", "not a framebuffer connection");
        }
        const size =
          node.options.type === "rdp" && node.options.width && node.options.height
            ? { w: node.options.width, h: node.options.height }
            : { w: width, h: height };
        // Plain VNC asks for a password only; VNC over TLS (VeNCrypt Plain) and RDP for user
        // name and password.
        const tls = isMockVncTls(node);
        const ask =
          hooks.decide(node, askCredentials) === "ask"
            ? {
                connection: node,
                kind:
                  node.options.type === "vnc" && !tls
                    ? ("password" as const)
                    : ("usernamePassword" as const),
                hooks,
              }
            : null;
        return fake.open(
          `${node.host}:${node.port ?? ""}`,
          size.w,
          size.h,
          onEvent,
          ask,
          node.options.type === "rdp" ? "rdp" : tls ? "vnc-tls" : "vnc",
          node.options.type === "rdp" && node.options.shareClipboard,
        );
      }),
    has: (id: SessionId): boolean => fake.has(id),
    credential: (id: SessionId, answer: CredentialAnswer, saveToVault: boolean) =>
      run("sessionCredential", () => fake.credential(id, answer, saveToVault)),
    credentialCancel: (id: SessionId) =>
      run("sessionCredentialCancel", () => fake.credentialCancel(id)),
    pointer: (id: SessionId, x: number, y: number, buttons: number, wheel: number) =>
      run("fbPointer", () => fake.pointer(id, x, y, buttons, wheel)),
    key: (id: SessionId, code: string, down: boolean) =>
      run("fbKey", () => fake.key(id, code, down)),
    resize: (id: SessionId, width: number, height: number) =>
      run("fbResize", () => fake.resize(id, width, height)),
    setVisible: (id: SessionId, visible: boolean) =>
      run("fbSetVisible", () => fake.setVisible(id, visible)),
    certDecision: (id: SessionId, accept: boolean) =>
      run("fbCertDecision", () => fake.certDecision(id, accept)),
    unencryptedDecision: (id: SessionId, allow: boolean) =>
      run("fbUnencryptedDecision", () => fake.unencryptedDecision(id, allow)),
    forgetCertificate: (id: SessionId) =>
      run("fbForgetCertificate", () => fake.forgetCertificate(id)),
    clipboardSave: (id: SessionId) => run("fbClipboardSave", () => fake.clipboardSave(id)),
    clipboardCancel: (id: SessionId) =>
      run("fbClipboardCancel", () => fake.clipboardCancel(id)),
    /** `true` when `id` was a mock framebuffer session (now closed). */
    close: (id: SessionId): boolean => fake.close(id),
  };
}
