import {
  toIpcError,
  type CredentialInput,
  type CredentialSummary,
  type Ipc,
  type IpcError,
  type RevealedSecret,
  type Uuid,
  type VaultStatus,
} from "../ipc";
import type { Result } from "./document.svelte";

/** How an unlock request ended. `manual` = "enter the credential by hand for this session". */
export type UnlockOutcome = "unlocked" | "manual" | "cancelled";

export interface UnlockRequest {
  /** Shown to the user, e.g. the name of the connection that needs a credential. */
  reason: string | null;
}

async function attempt<T>(run: () => Promise<T>): Promise<Result<T>> {
  try {
    return { ok: true, value: await run() };
  } catch (e) {
    return { ok: false, error: toIpcError(e) };
  }
}

/**
 * Vault state as seen by the UI. Holds only status and credential summaries (no secrets).
 * Secrets pass through method arguments or `reveal()` results and are never stored here.
 */
export class VaultStore {
  status = $state<VaultStatus | "unknown">("unknown");
  credentials = $state<CredentialSummary[]>([]);
  /** Set while the unlock/create screen is requested; rendered by the app shell. */
  request = $state<UnlockRequest | null>(null);
  /** The pending request also accepts "enter manually for this session". */
  manualAllowed = $state(false);
  #pending: ((outcome: UnlockOutcome) => void) | null = null;
  readonly #ipc: Ipc;

  constructor(ipc: Ipc) {
    this.#ipc = ipc;
  }

  get unlocked(): boolean {
    return this.status === "unlocked";
  }

  /** Reads the status from the backend (also catches an automatic lock) and the list. */
  async refresh(): Promise<Result<VaultStatus>> {
    const result = await attempt(() => this.#ipc.vaultStatus());
    if (!result.ok) return result;
    this.status = result.value;
    if (result.value === "unlocked") await this.#loadCredentials();
    else this.credentials = [];
    return result;
  }

  async #loadCredentials(): Promise<Result<CredentialSummary[]>> {
    const result = await attempt(() => this.#ipc.vaultList());
    if (result.ok) this.credentials = result.value;
    else this.#noteError(result.error);
    return result;
  }

  /** A `vault_locked` error from any call means the vault was locked behind our back. */
  #noteError(error: IpcError): void {
    if (error.code === "vault_locked") {
      this.status = "locked";
      this.credentials = [];
    }
  }

  async create(masterPassword: string): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.vaultCreate(masterPassword));
    if (result.ok) await this.refresh();
    return result;
  }

  async unlock(masterPassword: string): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.vaultUnlock(masterPassword));
    if (result.ok) await this.refresh();
    return result;
  }

  async lock(): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.vaultLock());
    if (result.ok) {
      this.status = "locked";
      this.credentials = [];
    }
    return result;
  }

  async changePassword(current: string, next: string): Promise<Result<void>> {
    return attempt(() => this.#ipc.vaultChangePassword(current, next));
  }

  async upsert(input: CredentialInput): Promise<Result<CredentialSummary>> {
    const result = await attempt(() => this.#ipc.vaultUpsert(input));
    if (result.ok) await this.#loadCredentials();
    else this.#noteError(result.error);
    return result;
  }

  /**
   * Updates a credential's metadata and keeps its secret: `secret: null` tells the backend to
   * leave the stored one alone, so the secret never travels to the UI for an edit.
   */
  async upsertKeepingSecret(
    base: Omit<CredentialInput, "secret"> & { id: Uuid },
  ): Promise<Result<CredentialSummary>> {
    return this.upsert({ ...base, secret: null });
  }

  /** The backend locked the vault (inactivity timer or `vault_lock`): drop what it guarded. */
  handleLocked(): void {
    this.status = "locked";
    this.credentials = [];
  }

  /** Starts listening to the backend's `vault-locked` event. Returns the unsubscribe. */
  listen(): () => void {
    return this.#ipc.onVaultLocked(() => this.handleLocked());
  }

  async remove(id: Uuid): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.vaultDelete(id));
    if (result.ok) await this.#loadCredentials();
    else this.#noteError(result.error);
    return result;
  }

  /** Explicit user action only. The caller owns the result and must drop it quickly. */
  async reveal(id: Uuid): Promise<Result<RevealedSecret>> {
    const result = await attempt(() => this.#ipc.vaultRevealSecret(id));
    if (!result.ok) this.#noteError(result.error);
    return result;
  }

  name(id: Uuid): string | null {
    return this.credentials.find((c) => c.id === id)?.name ?? null;
  }

  /**
   * Resolves true once the vault is unlocked. If it is not, asks the shell to show the
   * unlock (or create) screen and resolves false when the user cancels.
   */
  async ensureUnlocked(reason: string | null = null): Promise<boolean> {
    return (await this.#ask(reason, false)) === "unlocked";
  }

  /**
   * Like `ensureUnlocked`, but the unlock screen also offers "enter manually for this
   * session": the credential is then typed in the connection's own prompt and the vault stays
   * locked. Never use it when there is no vault: that case needs no unlock at all.
   */
  ensureUnlockedOrManual(reason: string | null = null): Promise<UnlockOutcome> {
    return this.#ask(reason, true);
  }

  #ask(reason: string | null, allowManual: boolean): Promise<UnlockOutcome> {
    if (this.unlocked) return Promise.resolve("unlocked");
    // A newer request supersedes (cancels) any request still pending.
    this.#pending?.("cancelled");
    this.request = { reason };
    this.manualAllowed = allowManual;
    return new Promise<UnlockOutcome>((resolve) => {
      this.#pending = resolve;
    });
  }

  /** Called by the unlock screen when it closes. */
  finishRequest(unlocked: boolean): void {
    this.#finish(unlocked ? "unlocked" : "cancelled");
  }

  /** The user chose "enter manually for this session". */
  finishRequestManual(): void {
    this.#finish("manual");
  }

  #finish(outcome: UnlockOutcome): void {
    const resolve = this.#pending;
    this.#pending = null;
    this.request = null;
    this.manualAllowed = false;
    resolve?.(outcome);
  }
}
