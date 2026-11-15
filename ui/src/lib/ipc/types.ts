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
  | "security_downgrade";

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
  | { type: "unencryptedPrompt" };

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
