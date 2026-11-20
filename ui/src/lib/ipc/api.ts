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
  QuickUnlockInfo,
  QuitReply,
  RevealedSecret,
  SessionId,
  TermEvent,
  UpdateCheck,
  UpdateEvent,
  UpdateSettings,
  Uuid,
  VaultStatus,
  WebNavAction,
  WhatsNew,
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
  /**
   * Moves a node under `newParentId` (null = root), right before its sibling `before` (same
   * kind, same new parent), or at the end of its group when `before` is null. The group gets
   * contiguous `sortIndex` values.
   * `visibleOrder` lists the ids of the target group's siblings as the UI shows them; the
   * backend uses it only while the group has no index yet, so nothing jumps on the first move.
   */
  nodeReorder(
    id: Uuid,
    newParentId: Uuid | null,
    before: Uuid | null,
    visibleOrder?: Uuid[],
  ): Promise<void>;
  /** Removes the manual order of the children of `parentId` (null = root). */
  nodeSortByName(parentId: Uuid | null): Promise<void>;

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
  /** Minutes of inactivity before the vault locks itself: 0 (never), 5, 15, 30 or 60. */
  vaultGetAutoLock(): Promise<number>;
  /** Saves the auto-lock time and applies it at once. Anything but the five choices is refused. */
  vaultSetAutoLock(minutes: number): Promise<void>;
  /**
   * Quick unlock with Touch ID / Windows Hello. `available`: the system has a usable biometric
   * method (show the toggle only then); `enabled`: the option is on; `ready`: a quick unlock can
   * be attempted now (on, and a key is held in memory since the last unlock with the password).
   */
  vaultQuickUnlockInfo(): Promise<QuickUnlockInfo>;
  /**
   * Turns quick unlock on or off. Off drops the held key at once. On does not create a key: it
   * is created by the next unlock with the master password (`ready` stays false until then).
   * Refused with `unsupported` when the system has no biometric method.
   */
  vaultSetQuickUnlock(enabled: boolean): Promise<void>;
  /**
   * Asks for Touch ID / Windows Hello and reopens the vault with the held key. Rejects with
   * `quick_unlock_cancelled` (the user dismissed the prompt), `quick_unlock_failed` (the check or
   * the unlock failed; the key may have been dropped) or `quick_unlock_not_ready` (no key held):
   * in every case the master password is the fallback.
   */
  vaultQuickUnlock(): Promise<void>;

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
  /**
   * Saves the files on the remote clipboard (RDP with a shared clipboard). The backend opens a
   * native folder dialog; the answer is `false` when the user dismissed it. Progress and the
   * outcome arrive as `clipboard` events.
   */
  fbClipboardSave(sessionId: SessionId): Promise<boolean>;
  /** Stops the save in progress. */
  fbClipboardCancel(sessionId: SessionId): Promise<void>;

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

  // updates (the backend does all the networking; the webview needs no network permission)
  updateSettings(): Promise<UpdateSettings>;
  updateSetAutoCheck(enabled: boolean): Promise<void>;
  /**
   * Looks for a newer version. `manual` (the "Check for updates…" action) ignores the automatic
   * setting and the skipped version and always answers; network errors reject.
   */
  updateCheck(manual: boolean): Promise<UpdateCheck>;
  /** "Skip this version": automatic checks stop proposing it. */
  updateSkip(version: string): Promise<void>;
  /**
   * Downloads the update found by the last `updateCheck`, verifies its signature, installs it and
   * restarts the application. The promise rejects with the reason when anything fails, and
   * normally never resolves because the application restarts.
   */
  updateInstall(onEvent: (event: UpdateEvent) => void): Promise<void>;
  /** The "What's new" due at this start-up (`null`: nothing to show). */
  whatsNew(): Promise<WhatsNew | null>;
  /** Opens the GitHub release page of `version` in the browser. */
  updateOpenReleasePage(version: string): Promise<void>;
  /** Opens the full changelog on GitHub in the browser. */
  updateOpenChangelog(): Promise<void>;
  /**
   * Answers `quit-requested`: `asking` (a confirmation is about to show; stops the backend's
   * 2 second timer), `quit` (nothing to ask or the user confirmed) or `cancel`.
   */
  quitReply(reply: QuitReply): Promise<void>;

  // backend notices (events, not commands). Each returns the function that unsubscribes.
  /** The vault locked itself (inactivity) or was locked: update the state at once. */
  onVaultLocked(handler: () => void): () => void;
  /** macOS menu bar: "Check for Updates…" was chosen (the event never fires elsewhere). */
  onCheckForUpdatesMenu(handler: () => void): () => void;
  /**
   * The user asked to quit (⌘Q, the menu, the Dock) or to close the window; the backend stopped
   * it and waits for `quitReply`. Without an answer within 2 seconds the application quits.
   */
  onQuitRequested(handler: () => void): () => void;
  /** macOS menu bar: "Close Tab" (⌘W) was chosen; the active tab is to be closed. */
  onCloseTabMenu(handler: () => void): () => void;
  /** "Save to the vault" failed at connect time; the session stays connected. */
  onCredentialSaveFailed(handler: (notice: CredentialSaveFailed) => void): () => void;

  // not part of the §4.3 contract: while a graphical session holds the keyboard with the Mac
  // shortcuts on, the native Edit menu is taken out of the macOS menu bar so that ⌘C, ⌘V, ⌘X,
  // ⌘A and ⌘Z reach the remote machine; `false` puts it back. No effect elsewhere.
  menuSetSessionKeys(captured: boolean): Promise<void>;

  // not part of the §4.3 contract: text on the system clipboard (read by the backend, so the
  // webview needs no clipboard permission); null when the clipboard holds no text.
  clipboardReadText(): Promise<string | null>;

  // not part of the §4.3 contract: lifecycle line in the backend log (never pass secrets)
  uiLog(level: "info" | "warn" | "error", message: string): Promise<void>;
}
