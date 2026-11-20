// In-memory implementation of `Ipc`: a document and a fake vault, so the UI can be developed
// without the backend. Session commands are not simulated and reject with `unsupported`.
// NOTHING here is secure: the "vault" keeps secrets in plain memory.

import { isNodeColor, isNodeIcon } from "../design/nodeAppearance";
import { createMockFramebuffer } from "../sessions/fb/mock-fb";
import { createMockWeb } from "../sessions/web/mock-web";
import { effectiveCredentialOf } from "../tree/model";
import type { Ipc } from "./api";
import { IpcError } from "./errors";
import { MockPrompter, type MockPromptHooks } from "./mock-credentials";
import { createMockTransfer, readMockKeyFile, type MockExport } from "./mock-transfer";
import { createMockUpdates, type MockUpdateOptions, type MockUpdates } from "./mock-updates";
import {
  ACCENT_CHOICES,
  SCHEME_CHOICES,
  DEFAULT_APPEARANCE,
  DENSITY_CHOICES,
  TERMINAL_FONT_SIZE_MAX,
  TERMINAL_FONT_SIZE_MIN,
  THEME_CHOICES,
} from "./types";
import type {
  Appearance,
  Connection,
  CredentialAnswer,
  CredentialInput,
  CredentialSaveFailed,
  CredentialSummary,
  Document,
  ErrorCode,
  Folder,
  KnownHostEntry,
  Node,
  QuitReply,
  RevealedSecret,
  SecretInput,
  TermEvent,
  Uuid,
  VaultStatus,
} from "./types";

interface StoredCredential {
  summary: CredentialSummary;
  secret: SecretInput;
}

function unsupported(what: string): Promise<never> {
  return Promise.reject(new IpcError("unsupported", `${what} is not available in the mock IPC`));
}

/** Master password of the demo vault (`createMockIpc({ vault: "locked" })`). Fake, mock only. */
export const MOCK_MASTER_PASSWORD = "portolano-demo";

const id = (n: number): Uuid => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;

/** Fixed ids of the demo content, so tests can refer to it. */
export const MOCK_IDS = {
  lab: id(1),
  localSshd: id(2),
  linux: id(3),
  buildBox: id(4),
  devVm: id(5),
  windows: id(6),
  rdpServer: id(7),
  rdpFixed: id(8),
  production: id(9),
  intranet: id(10),
  publicSite: id(11),
  localZsh: id(12),
  credLab: id(101),
  credWindows: id(102),
  credKey: id(103),
} as const;

function seedCredentials(): StoredCredential[] {
  return [
    {
      summary: { id: MOCK_IDS.credLab, name: "Lab admin", username: "admin", domain: null, kind: "password" },
      secret: { type: "password", password: "mock-lab-password" },
    },
    {
      summary: {
        id: MOCK_IDS.credWindows,
        name: "Windows domain user",
        username: "svc-portolano",
        domain: "LAB",
        kind: "password",
      },
      secret: { type: "password", password: "mock-windows-password" },
    },
    {
      summary: { id: MOCK_IDS.credKey, name: "Deploy key", username: "deploy", domain: null, kind: "sshKey" },
      secret: {
        type: "sshKey",
        privateKeyPem: "-----BEGIN MOCK KEY-----\nnot-a-real-key\n-----END MOCK KEY-----",
        passphrase: null,
      },
    },
  ];
}

function seedDocument(withCredentials: boolean): Document {
  const folder = (
    fid: Uuid,
    parentId: Uuid | null,
    name: string,
    credential: Folder["credential"],
  ): Node => ({ type: "folder", id: fid, parentId, name, credential });
  const connection = (
    cid: Uuid,
    parentId: Uuid | null,
    name: string,
    host: string,
    port: number | null,
    options: Connection["options"],
    extra: Partial<Connection> = {},
  ): Node => ({
    type: "connection",
    id: cid,
    parentId,
    name,
    host,
    port,
    credential: { type: "inherit" },
    username: null,
    tags: [],
    notes: "",
    options,
    ...extra,
  });
  const credOrInherit = (cid: Uuid): Folder["credential"] =>
    withCredentials ? { type: "id", id: cid } : { type: "inherit" };

  return {
    schemaVersion: 1,
    nodes: [
      // The first node must stay a folder (existing tests rely on it).
      folder(MOCK_IDS.lab, null, "Lab", credOrInherit(MOCK_IDS.credLab)),
      connection(MOCK_IDS.localSshd, MOCK_IDS.lab, "Local sshd", "127.0.0.1", 12222, {
        type: "ssh",
        jumpHost: null,
        keepaliveSecs: 30,
      }, { tags: ["dev"] }),
      folder(MOCK_IDS.linux, MOCK_IDS.lab, "Linux", { type: "inherit" }),
      connection(MOCK_IDS.buildBox, MOCK_IDS.linux, "Build box", "build.lab.example", null, {
        type: "ssh",
        jumpHost: MOCK_IDS.localSshd,
        keepaliveSecs: null,
      }, { tags: ["ci", "dev"], notes: "Reached through the local sshd." }),
      connection(MOCK_IDS.devVm, MOCK_IDS.linux, "Dev VM (VNC)", "10.0.0.20", 5901, {
        type: "vnc",
        viewOnly: false,
      }, { credential: withCredentials ? { type: "id", id: MOCK_IDS.credKey } : { type: "none" } }),
      folder(MOCK_IDS.windows, MOCK_IDS.lab, "Windows", credOrInherit(MOCK_IDS.credWindows)),
      connection(MOCK_IDS.rdpServer, MOCK_IDS.windows, "File server", "fs01.lab.example", null, {
        type: "rdp",
        width: null,
        height: null,
        domain: "LAB",
        ignoreCert: false,
        allowWithoutNla: false,
        kerberosKdc: null,
        shareClipboard: false,
      macShortcuts: false,
      }, { tags: ["windows"] }),
      connection(MOCK_IDS.rdpFixed, MOCK_IDS.windows, "Kiosk (1280x800)", "kiosk.lab.example", 3390, {
        type: "rdp",
        width: 1280,
        height: 800,
        domain: null,
        ignoreCert: true,
        allowWithoutNla: false,
        kerberosKdc: null,
        shareClipboard: false,
      macShortcuts: false,
      }),
      folder(MOCK_IDS.production, null, "Production", { type: "none" }),
      connection(MOCK_IDS.intranet, MOCK_IDS.production, "Intranet", "intranet.example", null, {
        type: "web",
        url: "https://intranet.example/",
        isolatedProfile: false,
      }, { tags: ["web"] }),
      connection(MOCK_IDS.publicSite, MOCK_IDS.production, "Public site (isolated)", "www.example", null, {
        type: "web",
        url: "https://www.example/admin",
        isolatedProfile: true,
      }),
      connection(MOCK_IDS.localZsh, null, "Local shell", "", null, {
        type: "localShell",
        program: null,
        args: [],
        cwd: null,
      }),
    ],
  };
}

// ---- fake terminal sessions (WP5) ------------------------------------------------------
// Opt-in (`createMockIpc({ terminal: true })`). An echo "shell" with the same event sequence as
// the real backend: SSH connections first emit `hostKeyPrompt` (unless the host key is known)
// and wait for `sshHostKeyDecision`; a host with a stored key that differs ends in
// `host_key_mismatch`. Nothing here talks to a network or runs a process.

/** The "server" key every mock host presents, derived from its name. */
const mockFingerprint = (host: string): string => `SHA256:mock-${host.toLowerCase()}`;

/** A key stored for `changed.example`, which does not match what the mock server presents. */
const MOCK_STALE_KEY: KnownHostEntry = {
  host: "changed.example",
  port: 22,
  algorithm: "ssh-ed25519",
  fingerprint: "SHA256:mock-stale-key-of-the-old-server",
};

interface MockTermSession {
  connection: Connection;
  onEvent: (event: TermEvent) => void;
  /** Set when the session asks the user for a credential. */
  prompter: MockPrompter | null;
  /** Resolves the pending host key prompt. */
  decide: ((accept: boolean) => void) | null;
  line: string;
  cols: number;
  rows: number;
  open: boolean;
}

interface MockTerminal {
  open(
    connectionId: Uuid,
    cols: number,
    rows: number,
    onEvent: (e: TermEvent) => void,
    askCredentials: boolean,
  ): Promise<string>;
  has(sessionId: string): boolean;
  credential(sessionId: string, answer: CredentialAnswer, saveToVault: boolean): Promise<void>;
  credentialCancel(sessionId: string): Promise<void>;
  write(sessionId: string, data: Uint8Array): Promise<void>;
  ack(sessionId: string, bytes: number): Promise<void>;
  resize(sessionId: string, cols: number, rows: number): Promise<void>;
  hostKeyDecision(sessionId: string, accept: boolean): Promise<void>;
  knownHostsList(): Promise<KnownHostEntry[]>;
  knownHostsRemove(host: string, port: number): Promise<void>;
  /** `true` when `sessionId` was a mock terminal session (now closed). */
  close(sessionId: string): boolean;
}

function createMockTerminal(
  getDoc: () => Document,
  enabled: boolean,
  hooks: MockPromptHooks,
): MockTerminal {
  if (!enabled) {
    return {
      open: () => unsupported("termOpen"),
      has: () => false,
      credential: () => unsupported("sessionCredential"),
      credentialCancel: () => unsupported("sessionCredentialCancel"),
      write: () => unsupported("termWrite"),
      ack: () => unsupported("termAck"),
      resize: () => unsupported("termResize"),
      hostKeyDecision: () => unsupported("sshHostKeyDecision"),
      knownHostsList: async () => [],
      knownHostsRemove: () => unsupported("knownHostsRemove"),
      close: () => false,
    };
  }

  const sessions = new Map<string, MockTermSession>();
  let counter = 0;
  let knownHosts: KnownHostEntry[] = [{ ...MOCK_STALE_KEY }];
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const session = (id: string): MockTermSession => {
    const found = sessions.get(id);
    if (!found || !found.open) throw new IpcError("not_found", `session ${id} not found`);
    return found;
  };
  const say = (s: MockTermSession, text: string): void =>
    s.onEvent({ type: "data", bytes: encoder.encode(text) });
  const prompt = (s: MockTermSession): void => say(s, "$ ");
  const fail = (s: MockTermSession, code: ErrorCode, message: string): void =>
    s.onEvent({ type: "error", error: { code, message } });

  const greet = (s: MockTermSession): void => {
    const where = s.connection.host === "" ? "local shell" : s.connection.host;
    say(s, `Portolano mock terminal on ${where}\r\nType a line to see it echoed; \`exit\` ends the session.\r\n`);
    prompt(s);
  };

  /** The host key is settled: ask for the credential if needed, then greet. */
  const authenticate = (s: MockTermSession): void => {
    if (s.prompter) s.prompter.ask();
    else greet(s);
  };

  const connect = (s: MockTermSession): void => {
    const { host } = s.connection;
    const port = s.connection.port ?? 22;
    const stored = knownHosts.filter((k) => k.host.toLowerCase() === host.toLowerCase() && k.port === port);
    const presented = mockFingerprint(host);
    if (stored.some((k) => k.fingerprint === presented)) {
      authenticate(s);
    } else if (stored.length > 0) {
      fail(
        s,
        "host_key_mismatch",
        `The host key of ${host}:${port} has CHANGED (possible man-in-the-middle attack). ` +
          `Stored: ${stored.map((k) => `${k.algorithm} ${k.fingerprint}`).join(", ")}. ` +
          `Presented: ssh-ed25519 ${presented}.`,
      );
    } else {
      s.decide = (accept) => {
        s.decide = null;
        if (!accept) {
          fail(s, "auth_failed", `The host key of ${host}:${port} was not accepted.`);
          return;
        }
        knownHosts = [...knownHosts, { host, port, algorithm: "ssh-ed25519", fingerprint: presented }];
        authenticate(s);
      };
      s.onEvent({ type: "hostKeyPrompt", fingerprint: presented, algorithm: "ssh-ed25519", host, port });
    }
  };

  const typed = (s: MockTermSession, text: string): void => {
    for (const ch of text) {
      if (ch === "\r") {
        const command = s.line.trim();
        s.line = "";
        say(s, "\r\n");
        if (command === "exit") {
          s.onEvent({ type: "exit", code: 0 });
          return;
        }
        if (command !== "") say(s, `mock: ${command}\r\n`);
        prompt(s);
      } else if (ch === "\x7f") {
        if (s.line.length > 0) {
          s.line = s.line.slice(0, -1);
          say(s, "\b \b");
        }
      } else if (ch >= " ") {
        s.line += ch;
        say(s, ch);
      }
    }
  };

  return {
    async open(connectionId, cols, rows, onEvent, askCredentials) {
      const node = getDoc().nodes.find((n) => n.id === connectionId);
      if (!node) throw new IpcError("not_found", `connection ${connectionId} not found`);
      if (node.type !== "connection") throw new IpcError("invalid_input", "not a connection");
      if (node.options.type !== "ssh" && node.options.type !== "localShell") {
        throw new IpcError("invalid_input", "not a terminal connection");
      }
      counter += 1;
      const id = `mock-term-${counter}`;
      const asks = node.options.type === "ssh" && hooks.decide(node, askCredentials) === "ask";
      const s: MockTermSession = {
        connection: node,
        onEvent,
        prompter: null,
        decide: null,
        line: "",
        cols,
        rows,
        open: true,
      };
      if (asks) {
        s.prompter = new MockPrompter(node, "sshKey", hooks, {
          emit: (prompt) => onEvent({ type: "credentialPrompt", ...prompt }),
          granted: () => greet(s),
          failed: (code, message) => fail(s, code, message),
        });
      }
      sessions.set(id, s);
      // Like the real backend, `open` returns at once and the events follow.
      setTimeout(() => {
        if (!s.open) return;
        if (node.options.type === "ssh") connect(s);
        else greet(s);
      }, 0);
      return id;
    },
    async write(sessionId, data) {
      const s = session(sessionId);
      // Still waiting for the host key decision or the credentials: input is discarded.
      if (s.decide || s.prompter?.waiting) return;
      typed(s, decoder.decode(data));
    },
    async ack(sessionId) {
      session(sessionId);
    },
    async resize(sessionId, cols, rows) {
      const s = session(sessionId);
      s.cols = cols;
      s.rows = rows;
    },
    async hostKeyDecision(sessionId, accept) {
      const s = session(sessionId);
      if (!s.decide) throw new IpcError("invalid_input", "no host key decision is pending");
      s.decide(accept);
    },
    has: (sessionId) => sessions.has(sessionId),
    async credential(sessionId, answer, saveToVault) {
      const s = session(sessionId);
      if (!s.prompter) throw new IpcError("invalid_input", "no credential prompt is pending");
      s.prompter.answer(answer, saveToVault);
    },
    async credentialCancel(sessionId) {
      const s = session(sessionId);
      if (!s.prompter) throw new IpcError("invalid_input", "no credential prompt is pending");
      s.prompter.cancel();
    },
    async knownHostsList() {
      return knownHosts.map((k) => ({ ...k }));
    },
    async knownHostsRemove(host, port) {
      knownHosts = knownHosts.filter((k) => !(k.host.toLowerCase() === host.toLowerCase() && k.port === port));
    },
    close(sessionId) {
      const s = sessions.get(sessionId);
      if (!s) return false;
      s.open = false;
      sessions.delete(sessionId);
      return true;
    },
  };
}

/** Ids of the folders (or connections) under `parent` in on-screen order: manual index first, then name. */
function siblingOrder(nodes: Node[], parent: Uuid | null, folders: boolean): Uuid[] {
  return nodes
    .filter((n) => n.parentId === parent && (n.type === "folder") === folders)
    .sort((a, b) => {
      const ai = a.sortIndex ?? Number.POSITIVE_INFINITY;
      const bi = b.sortIndex ?? Number.POSITIVE_INFINITY;
      if (ai !== bi) return ai < bi ? -1 : 1;
      return a.name.localeCompare(b.name, undefined, { sensitivity: "base", numeric: true });
    })
    .map((n) => n.id);
}

function descendantIds(nodes: Node[], rootId: Uuid): Set<Uuid> {
  const ids = new Set<Uuid>([rootId]);
  let grew = true;
  while (grew) {
    grew = false;
    for (const n of nodes) {
      if (n.parentId !== null && ids.has(n.parentId) && !ids.has(n.id)) {
        ids.add(n.id);
        grew = true;
      }
    }
  }
  return ids;
}

/** Creates a fresh mock; each call has its own state (use one per test). */
export interface MockOptions {
  /** `false` starts with an empty document (default: demo content). */
  seed?: boolean;
  /**
   * Initial vault: "notCreated" (default), or "locked" / "unlocked" with the demo credentials
   * and `MOCK_MASTER_PASSWORD`. The seeded folders reference those credentials.
   */
  vault?: VaultStatus;
  /**
   * `true` enables the fake terminal sessions (echo shell, host key prompt, changed-key demo).
   * Default `false`: the terminal commands reject with `unsupported`.
   */
  terminal?: boolean;
  /**
   * `true` enables the fake framebuffer sessions (test pattern, pointer and key echo, certificate
   * prompt). Default `false`: the `fb*` commands reject with `unsupported`.
   */
  framebuffer?: boolean;
  /**
   * `true` enables the fake web sessions (no real webview: the commands validate and track
   * state only). Default `false`: the `web*` commands reject with `unsupported`.
   */
  web?: boolean;
  /** Auto-lock time the mock starts with, in minutes (default 15). */
  autoLockMinutes?: number;
  /** Whether the pretend system has Touch ID / Windows Hello (default `true`). */
  quickUnlockAvailable?: boolean;
  /** The saved appearance at the start (default: system theme, portolano scheme, blue, comfortable, 13 px). */
  appearance?: Appearance;
  /** Canned answers of the update commands (default: up to date, no "What's new"). */
  updates?: MockUpdateOptions;
}

/** Knobs and counters of the fake quick unlock (what the biometric prompt answers). */
export interface MockQuickUnlock {
  /**
   * What the next `vaultQuickUnlock` does: `ok` opens the vault; `cancelled` and `failed` are the
   * prompt's answers (the key stays); `unlock-failed` is a stale key (`quick_unlock_failed`, key
   * dropped).
   */
  outcome: "ok" | "cancelled" | "failed" | "unlock-failed";
  /** How many biometric prompts were shown. */
  prompts: number;
}

/** The mock plus what tests need to play the backend's part. */
export interface MockIpc extends Ipc {
  /** Plays the backend: events that arrive without any UI call. */
  readonly simulate: {
    /** The vault locks itself (inactivity timer): status changes and `vault-locked` is emitted. */
    autoLockVault(): void;
    /** A "save to the vault" failure after a successful connection. */
    credentialSaveFailed(notice: CredentialSaveFailed): void;
    /** "Check for Updates…" chosen in the macOS menu bar. */
    checkForUpdatesMenu(): void;
    /** "Close Tab" (⌘W) chosen in the macOS menu bar. */
    closeTabMenu(): void;
    /** "Settings…" (⌘,) chosen in the macOS menu bar. */
    settingsMenu(): void;
    /** The user asked to quit or to close the window (`quit-requested`). */
    quitRequested(): void;
  };
  /** Every `menuSetSessionKeys` call, in order. */
  readonly sessionKeyCaptures: readonly boolean[];
  /** Every `quitReply` call, in order. */
  readonly quitReplies: readonly QuitReply[];
  /** Every successful `exportDocument` call. */
  readonly exports: readonly MockExport[];
  /** What the update commands received, and the knobs to steer their answers. */
  readonly updates: MockUpdates;
  /** The fake biometric prompt of quick unlock. */
  readonly quickUnlock: MockQuickUnlock;
}

export function createMockIpc(options: MockOptions = {}): MockIpc {
  const initialVault = options.vault ?? "notCreated";
  let doc: Document =
    options.seed === false
      ? { schemaVersion: 1, nodes: [] }
      : seedDocument(initialVault !== "notCreated");
  let vaultStatus: VaultStatus = initialVault;
  let autoLockMinutes = options.autoLockMinutes ?? 15;
  let masterPassword: string | null = initialVault === "notCreated" ? null : MOCK_MASTER_PASSWORD;
  const credentials = new Map<Uuid, StoredCredential>(
    initialVault === "notCreated" ? [] : seedCredentials().map((c) => [c.summary.id, c]),
  );

  const requireUnlocked = (): void => {
    if (vaultStatus !== "unlocked") throw new IpcError("vault_locked", "the vault is locked");
  };
  const requireNode = (id: Uuid): Node => {
    const node = doc.nodes.find((n) => n.id === id);
    if (!node) throw new IpcError("not_found", `node ${id} not found`);
    return node;
  };
  const clone = <T>(value: T): T => structuredClone(value);
  const lockedHandlers = new Set<() => void>();
  const menuUpdateHandlers = new Set<() => void>();
  const quitHandlers = new Set<() => void>();
  const quitReplies: QuitReply[] = [];
  const sessionKeyCaptures: boolean[] = [];
  const menuCloseTabHandlers = new Set<() => void>();
  const menuSettingsHandlers = new Set<() => void>();
  let appearance: Appearance = { ...(options.appearance ?? DEFAULT_APPEARANCE) };
  const saveFailedHandlers = new Set<(notice: CredentialSaveFailed) => void>();
  const lockVault = (): void => {
    if (vaultStatus !== "unlocked") return;
    vaultStatus = "locked";
    for (const handler of [...lockedHandlers]) handler();
  };
  const updates = createMockUpdates(options.updates);
  // Quick unlock mirrors the backend: a key is held only after a password unlock with the option on.
  const quickUnlock: MockQuickUnlock = { outcome: "ok", prompts: 0 };
  let quickEnabled = false;
  let quickHeld = false;
  const transfer = createMockTransfer(
    () => doc,
    (next) => {
      doc = next;
    },
  );
  /** The backend clears what pointed at deleted nodes; the mock does the same. */
  const clearJumpHosts = (doomed: Set<Uuid>): void => {
    doc = {
      ...doc,
      nodes: doc.nodes.map((n) =>
        n.type === "connection" &&
        n.options.type === "ssh" &&
        n.options.jumpHost !== null &&
        doomed.has(n.options.jumpHost)
          ? { ...n, options: { ...n.options, jumpHost: null } }
          : n,
      ),
    };
  };
  const clearCredentialRefs = (credentialId: Uuid): void => {
    doc = {
      ...doc,
      nodes: doc.nodes.map((n) =>
        n.credential.type === "id" && n.credential.id === credentialId
          ? { ...n, credential: { type: "none" } }
          : n,
      ),
    };
  };
  // How a session gets its credential, like the backend: the vault only when the connection
  // has one that exists; otherwise (or when asked to) the user is prompted. The vault is
  // optional: a mock without one is fully usable.
  const promptHooks: MockPromptHooks = {
    decide(connection, askRequested) {
      if (askRequested) return "ask";
      const effective = effectiveCredentialOf(doc.nodes, connection.id);
      if (effective.kind !== "id") return "ask";
      if (vaultStatus === "notCreated") return "ask";
      if (vaultStatus === "locked") throw new IpcError("vault_locked", "the vault is locked");
      return credentials.has(effective.id) ? "vault" : "ask";
    },
    canSave: () => vaultStatus === "unlocked",
    saved(connection, answer) {
      const credentialId = crypto.randomUUID();
      const secret: SecretInput =
        answer.keyPath !== null && answer.keyPath !== ""
          ? // The mock does not read files: it stores a placeholder, never the path's content.
            { type: "sshKey", privateKeyPem: "mock key file content", passphrase: answer.passphrase }
          : { type: "password", password: answer.password ?? "" };
      credentials.set(credentialId, {
        summary: {
          id: credentialId,
          name: `${connection.name} (${answer.username})`,
          username: answer.username,
          domain: answer.domain,
          kind: secret.type,
        },
        secret,
      });
      doc = {
        ...doc,
        nodes: doc.nodes.map((n) =>
          n.id === connection.id && n.type === "connection"
            ? { ...n, credential: { type: "id", id: credentialId } }
            : n,
        ),
      };
    },
  };
  const terminal = createMockTerminal(() => doc, options.terminal === true, promptHooks);
  const fb = createMockFramebuffer(() => doc, options.framebuffer === true, promptHooks);
  const web = createMockWeb(() => doc, options.web === true);

  return {
    simulate: {
      autoLockVault: lockVault,
      credentialSaveFailed(notice) {
        for (const handler of [...saveFailedHandlers]) handler(notice);
      },
      checkForUpdatesMenu() {
        for (const handler of [...menuUpdateHandlers]) handler();
      },
      closeTabMenu() {
        for (const handler of [...menuCloseTabHandlers]) handler();
      },
      settingsMenu() {
        for (const handler of [...menuSettingsHandlers]) handler();
      },
      quitRequested() {
        for (const handler of [...quitHandlers]) handler();
      },
    },
    quitReplies,
    sessionKeyCaptures,
    exports: transfer.exports,
    updates,
    quickUnlock,
    async docGet() {
      return clone(doc);
    },
    async docUpsertNode(node) {
      if (node.name.trim() === "") throw new IpcError("invalid_input", "the name is empty");
      // Like the backend: only the icons and colours of appearance.json are accepted on save.
      if (node.icon !== undefined && !isNodeIcon(node.icon)) {
        throw new IpcError("invalid_input", `unknown icon: ${node.icon}`);
      }
      if (node.color !== undefined && !isNodeColor(node.color)) {
        throw new IpcError("invalid_input", `unknown colour: ${node.color}`);
      }
      if (node.parentId !== null) {
        const parent = requireNode(node.parentId);
        if (parent.type !== "folder") {
          throw new IpcError("invalid_input", "the parent is not a folder");
        }
        if (descendantIds(doc.nodes, node.id).has(node.parentId)) {
          throw new IpcError("invalid_input", "a node cannot be moved into itself");
        }
      }
      const stored = clone(node);
      const index = doc.nodes.findIndex((n) => n.id === node.id);
      const nodes = doc.nodes.slice();
      if (index >= 0) {
        // Order is owned by nodeReorder / nodeSortByName: an edit keeps the stored index,
        // unless the node changes parent (then it joins the unindexed nodes of its new group).
        const old = nodes[index];
        const kept = old && old.parentId === node.parentId ? old.sortIndex : undefined;
        delete stored.sortIndex;
        if (kept !== undefined) stored.sortIndex = kept;
        nodes[index] = stored;
      } else nodes.push(stored);
      doc = { ...doc, nodes };
      return clone(stored);
    },
    async docDeleteNode(id) {
      requireNode(id);
      const doomed = descendantIds(doc.nodes, id);
      doc = { ...doc, nodes: doc.nodes.filter((n) => !doomed.has(n.id)) };
      clearJumpHosts(doomed);
    },
    async docMoveNode(id, newParentId) {
      const node = requireNode(id);
      if (newParentId !== null) {
        const parent = requireNode(newParentId);
        if (parent.type !== "folder") {
          throw new IpcError("invalid_input", "the new parent is not a folder");
        }
        if (descendantIds(doc.nodes, id).has(newParentId)) {
          throw new IpcError("invalid_input", "a node cannot be moved into itself");
        }
      }
      doc = {
        ...doc,
        nodes: doc.nodes.map((n) => (n.id === node.id ? { ...n, parentId: newParentId } : n)),
      };
    },
    async nodeReorder(id, newParentId, before, visibleOrder) {
      const node = requireNode(id);
      if (newParentId !== null) {
        const parent = requireNode(newParentId);
        if (parent.type !== "folder") {
          throw new IpcError("invalid_input", "the new parent is not a folder");
        }
        if (descendantIds(doc.nodes, id).has(newParentId)) {
          throw new IpcError("invalid_input", "a node cannot be moved into itself");
        }
      }
      if (before !== null) {
        const target = requireNode(before);
        if (
          before === id ||
          target.parentId !== newParentId ||
          target.type !== node.type
        ) {
          throw new IpcError(
            "invalid_input",
            "the node to move before must be a sibling of the same kind",
          );
        }
      }
      let group = siblingOrder(doc.nodes, newParentId, node.type === "folder");
      if (visibleOrder && group.every((g) => doc.nodes.find((n) => n.id === g)?.sortIndex === undefined)) {
        const members = new Set(group);
        const hinted = [...new Set(visibleOrder)].filter((h) => members.has(h));
        group = [...hinted, ...group.filter((g) => !hinted.includes(g))];
      }
      group = group.filter((other) => other !== id);
      group.splice(before === null ? group.length : group.indexOf(before), 0, id);
      const indices = new Map(group.map((nodeId, index) => [nodeId, index]));
      doc = {
        ...doc,
        nodes: doc.nodes.map((n) => {
          const index = indices.get(n.id);
          if (n.id === id) return { ...n, parentId: newParentId, sortIndex: index };
          return index === undefined ? n : { ...n, sortIndex: index };
        }),
      };
    },
    async nodeSortByName(parentId) {
      if (parentId !== null && requireNode(parentId).type !== "folder") {
        throw new IpcError("invalid_input", "the parent is not a folder");
      }
      doc = {
        ...doc,
        nodes: doc.nodes.map((n) => {
          if (n.parentId !== parentId) return n;
          const { sortIndex: _removed, ...rest } = n;
          return rest as Node;
        }),
      };
    },

    async vaultStatus() {
      return vaultStatus;
    },
    async vaultCreate(password) {
      if (vaultStatus !== "notCreated") throw new IpcError("invalid_input", "vault exists");
      if (password.length === 0) throw new IpcError("invalid_input", "empty master password");
      masterPassword = password;
      vaultStatus = "unlocked";
    },
    async vaultUnlock(password) {
      if (vaultStatus === "notCreated") throw new IpcError("invalid_input", "no vault yet");
      if (password !== masterPassword) throw new IpcError("auth_failed", "wrong master password");
      vaultStatus = "unlocked";
      quickHeld = quickEnabled;
    },
    async vaultChangePassword(current, next) {
      if (current !== masterPassword) throw new IpcError("auth_failed", "wrong master password");
      if (next.length === 0) throw new IpcError("invalid_input", "empty master password");
      masterPassword = next;
      quickHeld = false;
    },
    async vaultLock() {
      lockVault();
    },
    async vaultList() {
      requireUnlocked();
      return [...credentials.values()].map((c) => clone(c.summary));
    },
    async vaultUpsert(input: CredentialInput) {
      requireUnlocked();
      const id = input.id ?? crypto.randomUUID();
      // `secret: null` keeps the stored secret; it is only valid for an existing credential.
      const secret = input.secret ?? credentials.get(id)?.secret;
      if (!secret) {
        throw new IpcError("invalid_input", "a secret is required to create a credential");
      }
      const summary: CredentialSummary = {
        id,
        name: input.name,
        username: input.username,
        domain: input.domain,
        kind: secret.type,
      };
      credentials.set(id, { summary, secret: clone(secret) });
      return clone(summary);
    },
    async vaultDelete(id) {
      requireUnlocked();
      if (!credentials.delete(id)) throw new IpcError("not_found", `credential ${id} not found`);
      clearCredentialRefs(id);
    },
    async vaultRevealSecret(id): Promise<RevealedSecret> {
      requireUnlocked();
      const stored = credentials.get(id);
      if (!stored) throw new IpcError("not_found", `credential ${id} not found`);
      return clone(stored.secret);
    },

    async vaultGetAutoLock() {
      return autoLockMinutes;
    },
    async vaultSetAutoLock(minutes) {
      if (![0, 5, 15, 30, 60].includes(minutes)) {
        throw new IpcError("invalid_input", "unsupported auto-lock time");
      }
      autoLockMinutes = minutes;
    },
    async appearanceGet() {
      return { ...appearance };
    },
    async appearanceSet(next) {
      // The same checks as the backend: a value outside the lists changes nothing.
      const valid =
        (THEME_CHOICES as readonly string[]).includes(next.theme) &&
        (SCHEME_CHOICES as readonly string[]).includes(next.scheme) &&
        (ACCENT_CHOICES as readonly string[]).includes(next.accent) &&
        (DENSITY_CHOICES as readonly string[]).includes(next.density) &&
        Number.isInteger(next.terminalFontSize) &&
        next.terminalFontSize >= TERMINAL_FONT_SIZE_MIN &&
        next.terminalFontSize <= TERMINAL_FONT_SIZE_MAX;
      if (!valid) throw new IpcError("invalid_input", "unsupported appearance");
      appearance = { ...next };
      return { ...appearance };
    },
    async vaultQuickUnlockInfo() {
      return {
        available: options.quickUnlockAvailable ?? true,
        enabled: quickEnabled,
        ready: quickEnabled && quickHeld,
      };
    },
    async vaultSetQuickUnlock(enabled) {
      if (enabled && !(options.quickUnlockAvailable ?? true)) {
        throw new IpcError("unsupported", "Touch ID / Windows Hello is not available");
      }
      if (!enabled) quickHeld = false;
      quickEnabled = enabled;
    },
    async vaultQuickUnlock() {
      if (!quickEnabled || !quickHeld) {
        throw new IpcError("quick_unlock_not_ready", "quick unlock needs the master password first");
      }
      quickUnlock.prompts += 1;
      if (quickUnlock.outcome === "cancelled") {
        throw new IpcError("quick_unlock_cancelled", "quick unlock was cancelled");
      }
      if (quickUnlock.outcome === "failed") {
        throw new IpcError("quick_unlock_failed", "the biometric check failed");
      }
      if (quickUnlock.outcome === "unlock-failed") {
        quickHeld = false;
        throw new IpcError("quick_unlock_failed", "quick unlock could not open the vault");
      }
      vaultStatus = "unlocked";
    },

    termOpen: (connectionId, cols, rows, onEvent, openOptions) =>
      terminal.open(connectionId, cols, rows, onEvent, openOptions?.askCredentials === true),
    termWrite: (sessionId, data) => terminal.write(sessionId, data),
    termAck: (sessionId, bytes) => terminal.ack(sessionId, bytes),
    termResize: (sessionId, cols, rows) => terminal.resize(sessionId, cols, rows),
    sshHostKeyDecision: (sessionId, accept) => terminal.hostKeyDecision(sessionId, accept),
    knownHostsList: () => terminal.knownHostsList(),
    knownHostsRemove: (host, port) => terminal.knownHostsRemove(host, port),

    fbOpen: (connectionId, width, height, onEvent, openOptions) =>
      fb.open(connectionId, width, height, onEvent, openOptions?.askCredentials === true),
    fbPointer: (sessionId, x, y, buttons, wheel) => fb.pointer(sessionId, x, y, buttons, wheel),
    fbKey: (sessionId, code, down) => fb.key(sessionId, code, down),
    fbResize: (sessionId, width, height) => fb.resize(sessionId, width, height),
    fbSetVisible: (sessionId, visible) => fb.setVisible(sessionId, visible),
    fbCertDecision: (sessionId, accept) => fb.certDecision(sessionId, accept),
    fbUnencryptedDecision: (sessionId, allow) => fb.unencryptedDecision(sessionId, allow),
    fbForgetCertificate: (sessionId) => fb.forgetCertificate(sessionId),
    fbClipboardSave: (sessionId) => fb.clipboardSave(sessionId),
    fbClipboardCancel: (sessionId) => fb.clipboardCancel(sessionId),

    webOpen: (connectionId, bounds) => web.open(connectionId, bounds),
    webSetBounds: (sessionId, bounds, visible) => web.setBounds(sessionId, bounds, visible),
    webNavigate: (sessionId, action) => web.navigate(sessionId, action),

    sessionClose: (sessionId) =>
      terminal.close(sessionId) || fb.close(sessionId) || web.close(sessionId)
        ? Promise.resolve()
        : unsupported("sessionClose"),

    sessionCredential: (sessionId, answer, saveToVault) => {
      if (terminal.has(sessionId)) return terminal.credential(sessionId, answer, saveToVault);
      if (fb.has(sessionId)) return fb.credential(sessionId, answer, saveToVault);
      return unsupported("sessionCredential");
    },
    sessionCredentialCancel: (sessionId) => {
      if (terminal.has(sessionId)) return terminal.credentialCancel(sessionId);
      if (fb.has(sessionId)) return fb.credentialCancel(sessionId);
      return unsupported("sessionCredentialCancel");
    },

    importPreview: (format, path) => transfer.preview(format, path),
    importApply: (previewId, targetFolderId) => transfer.apply(previewId, targetFolderId),
    exportDocument: (format, path, nodeIds) => transfer.export(format, path, nodeIds),
    readPrivateKeyFile: (path) => readMockKeyFile(path),

    ...updates.api,

    onVaultLocked(handler) {
      lockedHandlers.add(handler);
      return () => lockedHandlers.delete(handler);
    },
    onCheckForUpdatesMenu(handler) {
      menuUpdateHandlers.add(handler);
      return () => menuUpdateHandlers.delete(handler);
    },
    onSettingsMenu(handler) {
      menuSettingsHandlers.add(handler);
      return () => menuSettingsHandlers.delete(handler);
    },
    onCloseTabMenu(handler) {
      menuCloseTabHandlers.add(handler);
      return () => menuCloseTabHandlers.delete(handler);
    },
    async menuSetSessionKeys(captured) {
      sessionKeyCaptures.push(captured);
    },
    onQuitRequested(handler) {
      quitHandlers.add(handler);
      return () => quitHandlers.delete(handler);
    },
    async quitReply(reply) {
      quitReplies.push(reply);
    },
    onCredentialSaveFailed(handler) {
      saveFailedHandlers.add(handler);
      return () => saveFailedHandlers.delete(handler);
    },

    async clipboardReadText() {
      // The mock has no system clipboard.
      return null;
    },

    async uiLog() {
      // The mock has no backend log.
    },
  };
}

/** Instance used by the browser demo (`pnpm dev:mock`): demo content and a locked vault. */
export const mockIpc: Ipc = createMockIpc({ vault: "locked", terminal: true, framebuffer: true, web: true });
