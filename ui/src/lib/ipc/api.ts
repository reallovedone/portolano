import type {
  Bounds,
  CredentialAnswer,
  CredentialInput,
  CredentialSaveFailed,
  CredentialSummary,
  Document,
  ExportFormat,
  FbEvent,
  ImportFormat,
  ImportPreview,
  KnownHostEntry,
  Node,
  OpenOptions,
  RevealedSecret,
  SessionId,
  TermEvent,
  Uuid,
  VaultStatus,
  WebNavAction,
} from "./types";

/**
 * One typed method per IPC command. Implemented by `tauri.ts` (real
 * backend) and `mock.ts` (in-memory). Every method rejects with an `IpcError`.
 */
export interface Ipc {
  // document
  docGet(): Promise<Document>;
  docUpsertNode(node: Node): Promise<Node>;
  docDeleteNode(id: Uuid): Promise<void>;
  docMoveNode(id: Uuid, newParentId: Uuid | null): Promise<void>;

  // vault
  vaultStatus(): Promise<VaultStatus>;
  vaultCreate(masterPassword: string): Promise<void>;
  vaultUnlock(masterPassword: string): Promise<void>;
  vaultChangePassword(currentPassword: string, newPassword: string): Promise<void>;
  vaultLock(): Promise<void>;
  vaultList(): Promise<CredentialSummary[]>;
  vaultUpsert(input: CredentialInput): Promise<CredentialSummary>;
  vaultDelete(id: Uuid): Promise<void>;
  vaultRevealSecret(id: Uuid): Promise<RevealedSecret>;

  // terminal family (local shell, SSH)
  termOpen(
    connectionId: Uuid,
    cols: number,
    rows: number,
    onEvent: (event: TermEvent) => void,
    options?: OpenOptions,
  ): Promise<SessionId>;
  /** Raw bytes as the request body. Send one at a time to preserve ordering. */
  termWrite(sessionId: SessionId, data: Uint8Array): Promise<void>;
  termAck(sessionId: SessionId, bytes: number): Promise<void>;
  termResize(sessionId: SessionId, cols: number, rows: number): Promise<void>;
  sshHostKeyDecision(sessionId: SessionId, accept: boolean): Promise<void>;
  knownHostsList(): Promise<KnownHostEntry[]>;
  knownHostsRemove(host: string, port: number): Promise<void>;

  // framebuffer family (RDP, VNC)
  fbOpen(
    connectionId: Uuid,
    width: number,
    height: number,
    onEvent: (event: FbEvent) => void,
    options?: OpenOptions,
  ): Promise<SessionId>;
  fbPointer(
    sessionId: SessionId,
    x: number,
    y: number,
    buttons: number,
    wheel: number,
  ): Promise<void>;
  /** `code` is `KeyboardEvent.code`. */
  fbKey(sessionId: SessionId, code: string, down: boolean): Promise<void>;
  fbResize(sessionId: SessionId, width: number, height: number): Promise<void>;
  fbSetVisible(sessionId: SessionId, visible: boolean): Promise<void>;
  fbCertDecision(sessionId: SessionId, accept: boolean): Promise<void>;
  /** Answer to an `unencryptedPrompt` (VNC): `true` agrees to send the password unencrypted. */
  fbUnencryptedDecision(sessionId: SessionId, allow: boolean): Promise<void>;
  /**
   * Removes the stored certificate of the session's host. The deliberate step after a changed
   * certificate or a refused downgrade; connect again afterwards.
   */
  fbForgetCertificate(sessionId: SessionId): Promise<void>;

  // web family
  webOpen(connectionId: Uuid, bounds: Bounds): Promise<SessionId>;
  webSetBounds(sessionId: SessionId, bounds: Bounds, visible: boolean): Promise<void>;
  webNavigate(sessionId: SessionId, action: WebNavAction): Promise<void>;

  // common
  sessionClose(sessionId: SessionId): Promise<void>;
  /**
   * Answers a `credentialPrompt` of a terminal or framebuffer session. The secrets go to the
   * backend and are used for that session only. The caller must clear its inputs afterwards.
   */
  sessionCredential(
    sessionId: SessionId,
    answer: CredentialAnswer,
    saveToVault: boolean,
  ): Promise<void>;
  /** The user dismissed the prompt: the session ends with the `cancelled` code. */
  sessionCredentialCancel(sessionId: SessionId): Promise<void>;

  // import / export
  importPreview(format: ImportFormat, path: string): Promise<ImportPreview>;
  importApply(previewId: Uuid, targetFolderId: Uuid | null): Promise<number>;
  exportDocument(format: ExportFormat, path: string, nodeIds: Uuid[] | null): Promise<void>;

  // key files
  /**
   * The text of the private key file at `path` (a path from the native open dialog). The
   * backend refuses anything but a small regular file holding a PEM/OpenSSH private key
   * (`invalid_input`; `not_found` / `io` when it cannot be read). The caller owns the result
   * and must drop it as soon as it is in a field.
   */
  readPrivateKeyFile(path: string): Promise<string>;

  // backend notices (events, not commands). Each returns the function that unsubscribes.
  /** The vault locked itself (inactivity) or was locked: update the state at once. */
  onVaultLocked(handler: () => void): () => void;
  /** "Save to the vault" failed at connect time; the session stays connected. */
  onCredentialSaveFailed(handler: (notice: CredentialSaveFailed) => void): () => void;

  // not part of the §4.3 contract: lifecycle line in the backend log (never pass secrets)
  uiLog(level: "info" | "warn" | "error", message: string): Promise<void>;
}
