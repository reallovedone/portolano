// TypeScript mirror of the Rust contract types.
// JSON convention (see crates/core-model/src/lib.rs): camelCase fields, internally tagged
// enums with a "type" property, plain camelCase strings for enums without data,
// optional values always present as null.

export type Uuid = string;
export type SessionId = Uuid;

// ---- core-model: errors ----

export type ErrorCode =
  | "vault_locked"
  | "auth_failed"
  | "host_unreachable"
  | "host_key_mismatch"
  | "not_found"
  | "invalid_input"
  | "io"
  | "unsupported"
  | "internal"
  /** The user dismissed a credential prompt (not an error to report). */
  | "cancelled"
  /** A host that used encryption before offers none now: the connection was refused. */
  | "security_downgrade"
  /** The user dismissed the Touch ID / Windows Hello request of quick unlock. */
  | "quick_unlock_cancelled"
  /** The biometric check failed or quick unlock could not open the vault: use the password. */
  | "quick_unlock_failed"
  /** Quick unlock holds no key (unlock with the password first). */
  | "quick_unlock_not_ready";

export interface AppError {
  code: ErrorCode;
  message: string;
}

// ---- core-model: document ----

export interface Document {
  schemaVersion: number;
  nodes: Node[];
}

export type Node = ({ type: "folder" } & Folder) | ({ type: "connection" } & Connection);

export interface Folder {
  id: Uuid;
  parentId: Uuid | null;
  name: string;
  credential: CredentialRef;
}

export interface Connection {
  id: Uuid;
  parentId: Uuid | null;
  name: string;
  host: string;
  port: number | null;
  credential: CredentialRef;
  /** Optional, non-secret user name that pre-fills the credential prompt. */
  username: string | null;
  tags: string[];
  notes: string;
  options: ConnectionOptions;
}

export type CredentialRef = { type: "inherit" } | { type: "none" } | { type: "id"; id: Uuid };

export type ConnectionOptions =
  | { type: "localShell"; program: string | null; args: string[]; cwd: string | null }
  | { type: "ssh"; jumpHost: Uuid | null; keepaliveSecs: number | null }
  | {
      type: "rdp";
      width: number | null;
      height: number | null;
      domain: string | null;
      ignoreCert: boolean;
      /**
       * Accept a server that only offers TLS-level security (no NLA) although a credential is
       * sent. The backend always writes it (false for documents saved before it existed).
       */
      allowWithoutNla: boolean;
      /**
       * Optional Kerberos KDC or KDC proxy for signing in with a domain account (`dc.example.com`,
       * `tcp://...`, `https://proxy/KdcProxy`). Null: found through DNS. Null for documents saved
       * before the option existed; the backend always writes it.
       */
      kerberosKdc: string | null;
      /**
       * Share the clipboard (text and files) with the remote desktop while the tab is the active
       * one. Off for documents saved before the option existed. The backend always writes it.
       */
      shareClipboard: boolean;
      /**
       * macOS only: send Cmd+key as Ctrl+key and a lone Cmd press as the Windows key. Off for
       * documents saved before the option existed. The backend always writes it.
       */
      macShortcuts: boolean;
    }
  | { type: "vnc"; viewOnly: boolean }
  | { type: "web"; url: string; isolatedProfile: boolean };

// ---- core-model: web ----

export interface Bounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type WebNavAction =
  | { type: "back" }
  | { type: "forward" }
  | { type: "reload" }
  | { type: "url"; url: string };

// ---- vault ----

export type VaultStatus = "notCreated" | "locked" | "unlocked";

/** What `vaultQuickUnlockInfo` reports (Touch ID / Windows Hello unlock). */
export interface QuickUnlockInfo {
  available: boolean;
  enabled: boolean;
  ready: boolean;
}
export type CredentialKind = "password" | "sshKey";

export interface CredentialSummary {
  id: Uuid;
  name: string;
  username: string;
  domain: string | null;
  kind: CredentialKind;
}

export type SecretInput =
  | { type: "password"; password: string }
  | { type: "sshKey"; privateKeyPem: string; passphrase: string | null };

export interface CredentialInput {
  id: Uuid | null;
  name: string;
  username: string;
  domain: string | null;
  /**
   * The new secret. `null` while editing an existing credential means "keep the stored one"
   * (the secret never has to be revealed to rename or re-label a credential); creating a
   * credential still requires it.
   */
  secret: SecretInput | null;
}

/** Output of `vault_reveal_secret`: the only place where secrets reach the UI. */
export type RevealedSecret =
  | { type: "password"; password: string }
  | { type: "sshKey"; privateKeyPem: string; passphrase: string | null };

// ---- backend notices (Tauri events to the main webview) ----

/** Answer to `quit-requested` (command `quit_reply`). */
export type QuitReply = "asking" | "quit" | "cancel";

/**
 * `credential-save-failed`: "save to the vault" failed after a successful connection (the
 * session stays connected). The message never contains the secret.
 */
export interface CredentialSaveFailed {
  connectionId: Uuid;
  connectionName: string;
  code: ErrorCode;
  message: string;
}

// ---- credential prompts (terminal and framebuffer families) ----

/**
 * Which fields the prompt asks for: `password` = a secret only (VNC); `usernamePassword` =
 * user name, password and an optional domain (RDP); `sshKey` = user name, then a password or
 * a private key file with an optional passphrase (SSH).
 */
export type CredentialPromptKind = "password" | "sshKey" | "usernamePassword";

/** Contains no secret. `error` says why the previous answer did not work. */
export interface CredentialPrompt {
  kind: CredentialPromptKind;
  usernameHint: string | null;
  domainHint: string | null;
  /** What the credential is for: `host:port` (for a jump host, the hop). */
  target: string;
  error: string | null;
  /** "Save to vault" may be offered (a vault exists and is unlocked). */
  canSaveToVault: boolean;
}

/**
 * What the user typed. Travels UI -> backend only. `keyPath` is a path: the backend reads the
 * file, its content never reaches the UI.
 */
export interface CredentialAnswer {
  username: string;
  password: string | null;
  keyPath: string | null;
  passphrase: string | null;
  domain: string | null;
}

/** Options of `termOpen` / `fbOpen`. */
export interface OpenOptions {
  /** Skip the vault for this session and ask the user ("enter manually for this session"). */
  askCredentials?: boolean;
}

// ---- session-term ----

export type TermEvent =
  | { type: "data"; bytes: Uint8Array }
  | {
      type: "hostKeyPrompt";
      fingerprint: string;
      algorithm: string;
      /** The hop this key belongs to: with jump hosts, not necessarily the final target. */
      host: string;
      port: number;
    }
  | { type: "exit"; code: number | null }
  | { type: "error"; error: AppError }
  | ({ type: "credentialPrompt" } & CredentialPrompt);

export interface KnownHostEntry {
  host: string;
  port: number;
  algorithm: string;
  fingerprint: string;
}

// ---- framebuffer family (session-rdp, session-vnc) ----

export type FbEvent =
  | { type: "resize"; w: number; h: number }
  | { type: "rect"; x: number; y: number; w: number; h: number; rgba: Uint8Array }
  | {
      type: "cursor";
      hotspotX: number;
      hotspotY: number;
      w: number;
      h: number;
      rgba: Uint8Array;
    }
  /**
   * `changed: false`: an unknown certificate, the user decides. `changed: true`: it differs from
   * the stored one (`stored`); the attempt has already ended and the only way forward is
   * `fbForgetCertificate` followed by a new connection.
   */
  | { type: "certPrompt"; fingerprint: string; changed: boolean; stored: string | null }
  | { type: "closed"; reason: string }
  | { type: "error"; error: AppError }
  | ({ type: "credentialPrompt" } & CredentialPrompt)
  /** VNC only, once, before the first `resize`: whether the connection is encrypted (TLS). */
  | { type: "security"; encrypted: boolean }
  /**
   * VNC only: the server offers no encryption and wants a password. Nothing derived from it was
   * sent yet; answer with `fbUnencryptedDecision`.
   */
  | { type: "unencryptedPrompt" }
  /** RDP with a shared clipboard: news about files on the remote clipboard and their saving. */
  | { type: "clipboard"; event: ClipboardEvent };

/**
 * Why a clipboard file save did not happen: `cancelled` (the user), `limit` (too many files or
 * bytes), `destination` (the chosen folder refused a file), `remote` (the remote machine could
 * not provide a file), `protocol` (it sent something invalid), `nothing` (no usable file).
 */
export type ClipboardFailure =
  | "cancelled"
  | "limit"
  | "destination"
  | "remote"
  | "protocol"
  | "nothing";

/** Counts and sizes only: file names and clipboard contents never reach the UI. */
export type ClipboardEvent =
  /** Files on the remote clipboard can be saved; `count: 0` means there are none (any more). */
  | { type: "remoteFiles"; count: number; bytes: number; skipped: number }
  | { type: "progress"; filesDone: number; filesTotal: number; bytesDone: number; bytesTotal: number }
  | { type: "done"; files: number; bytes: number; skipped: number }
  | { type: "failed"; reason: ClipboardFailure };

/** Bit mask of `fbPointer.buttons` (same bits as DOM `MouseEvent.buttons`). */
export const FB_BUTTON_LEFT = 1;
export const FB_BUTTON_RIGHT = 2;
export const FB_BUTTON_MIDDLE = 4;

// ---- importers ----

export type ImportFormat = "sshConfig" | "rdp" | "mobaxterm" | "mremoteng" | "csv" | "json";
export type ExportFormat = "json" | "rdp";

export interface ImportWarning {
  message: string;
  line: number | null;
}

export interface ImportPreview {
  previewId: Uuid;
  nodes: Node[];
  warnings: ImportWarning[];
}

// ---- updates (WP13a) ----

export interface UpdateSettings {
  /** "Automatically check for updates". */
  autoCheck: boolean;
  currentVersion: string;
}

/** An available update. `notes` is Markdown from the network: render it as text, never as HTML. */
export interface UpdateInfo {
  currentVersion: string;
  version: string;
  /** `null`: no notes could be fetched; offer the release page instead. */
  notes: string | null;
}

export type UpdateCheck =
  | { status: "upToDate" }
  /** An automatic check while the setting is off. */
  | { status: "disabled" }
  /** An automatic check found the version the user chose to skip. */
  | { status: "skipped"; version: string }
  | { status: "available"; update: UpdateInfo };

export type UpdateEvent =
  | { type: "progress"; downloaded: number; total: number | null }
  /** Downloaded and verified: installing (the application restarts right after). */
  | { type: "installing" };

/** "What's new in <version>", shown once on the first launch after an update. */
export interface WhatsNew {
  version: string;
  notes: string | null;
}
