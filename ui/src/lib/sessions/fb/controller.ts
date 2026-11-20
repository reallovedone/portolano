// Protocol-agnostic controller of a framebuffer session: it consumes `FbEvent`s and drives
// the `fb_*` commands. It knows nothing about RDP or VNC. The Svelte view owns the DOM and
// forwards DOM events; this class holds everything testable without a DOM.

import type { AppError, CredentialAnswer, CredentialPrompt, FbEvent, SecurityAuth } from "../../ipc";
import {
  CLIPBOARD_IDLE,
  dismissNotice,
  failedLocally,
  reduceClipboard,
  startSaving,
  type ClipboardUi,
} from "./clipboard";
import { cursorStyle, type ToDataUrl } from "./cursor";
import type { Size } from "./geometry";
import { InputController, type Geometry, type InputApi, type InputOptions } from "./input";
import { FrameRenderer, type RendererOptions, type Surface } from "./renderer";

/** The commands of the framebuffer family the controller uses (a subset of `Ipc`). */
export interface FbApi extends InputApi {
  resize(width: number, height: number): Promise<void>;
  setVisible(visible: boolean): Promise<void>;
  certDecision(accept: boolean): Promise<void>;
  /** Answer to an unencrypted-connection prompt (VNC). */
  unencryptedDecision(allow: boolean): Promise<void>;
  /** Removes the stored certificate of the session's host. */
  forgetCertificate(): Promise<void>;
  /** Answer to a credential prompt (`session_credential`). */
  credential(answer: CredentialAnswer, saveToVault: boolean): Promise<void>;
  /** The user dismissed the credential prompt (`session_credential_cancel`). */
  credentialCancel(): Promise<void>;
  /** Saves the files on the remote clipboard; `false`: the user dismissed the folder dialog. */
  clipboardSave(): Promise<boolean>;
  /** Stops the save in progress. */
  clipboardCancel(): Promise<void>;
}

export type Phase = "connecting" | "live" | "closed" | "cancelled";

export interface CertPrompt {
  fingerprint: string;
  /** The certificate differs from the stored one: the attempt has ended, nothing to decide. */
  changed: boolean;
  /** The fingerprint on file, when `changed`. */
  stored: string | null;
}

/** What the backend said about the transport: `unknown` until a `security` event arrives
 *  (RDP never sends one: it is always TLS). */
export type Transport = "unknown" | "encrypted" | "clear";

export interface FbViewState {
  phase: Phase;
  transport: Transport;
  /** How an RDP login was authenticated, when the backend said so (never sent by VNC). */
  auth: SecurityAuth | null;
  remote: Size;
  cert: CertPrompt | null;
  /** The server offers no encryption and wants a password: the user decides before anything is
   *  sent. */
  unencryptedPrompt: boolean;
  /** A host that used encryption before offers none now (the message of the refusal). */
  downgrade: string | null;
  /** Set while the backend waits for credentials typed by the user. Holds no secret. */
  credential: CredentialPrompt | null;
  closedReason: string | null;
  cursor: string;
  /** Files on the remote clipboard and the save of them (RDP with a shared clipboard). */
  clipboard: ClipboardUi;
}

export interface ControllerOptions {
  api: FbApi;
  surface: Surface;
  /** Resizes the canvas bitmap (which clears it). */
  setCanvasSize: (w: number, h: number) => void;
  onState: (state: FbViewState) => void;
  /** An `FbEvent.error`: the shell shows it in the tab. */
  onFatal: (error: AppError) => void;
  /** Failures of fire-and-forget calls (never contain secrets). */
  onError: (what: string, error: unknown) => void;
  /** The session got going after the typed credential was asked to be saved to the vault. */
  onCredentialSaved?: () => void;
  geometry: () => Geometry | null;
  onEscape?: () => void;
  /** Connect again (after the stored certificate was removed). */
  onReconnect?: () => void;
  renderer?: RendererOptions;
  input?: Pick<InputOptions, "requestFrame" | "cancelFrame" | "setTimer" | "clearTimer" | "macShortcuts">;
  setTimer?: (callback: () => void, ms: number) => number;
  clearTimer?: (handle: number) => void;
  toDataUrl?: ToDataUrl;
  resizeDebounceMs?: number;
}

export class FbController {
  readonly input: InputController;
  readonly renderer: FrameRenderer;
  #state: FbViewState = {
    phase: "connecting",
    transport: "unknown",
    auth: null,
    remote: { w: 0, h: 0 },
    cert: null,
    unencryptedPrompt: false,
    downgrade: null,
    credential: null,
    closedReason: null,
    cursor: "default",
    clipboard: CLIPBOARD_IDLE,
  };
  /** The last answer asked to be saved; cleared once the session proves it worked. */
  #saveRequested = false;
  #visible: boolean | null = null;
  #resizeTimer: number | null = null;
  #lastRequested: Size | null = null;
  #disposed = false;
  readonly #o: ControllerOptions;

  constructor(options: ControllerOptions) {
    this.#o = options;
    this.renderer = new FrameRenderer(options.surface, options.renderer);
    this.input = new InputController({
      api: options.api,
      geometry: options.geometry,
      onEscape: options.onEscape,
      onError: (e) => options.onError("input", e),
      ...options.input,
    });
  }

  get state(): FbViewState {
    return this.#state;
  }

  #patch(patch: Partial<FbViewState>): void {
    this.#state = { ...this.#state, ...patch };
    this.#o.onState(this.#state);
  }

  /** Feeds one event of the session. */
  handleEvent(event: FbEvent): void {
    if (this.#disposed) return;
    switch (event.type) {
      case "resize":
        // The bitmap is cleared by the size change: drop what was queued for the old size.
        this.renderer.discard();
        this.#o.setCanvasSize(event.w, event.h);
        this.#patch({
          remote: { w: event.w, h: event.h },
          phase: "live",
          cert: null,
          unencryptedPrompt: false,
          credential: null,
        });
        if (this.#saveRequested) {
          this.#saveRequested = false;
          this.#o.onCredentialSaved?.();
        }
        break;
      case "rect":
        this.renderer.enqueue(event);
        break;
      case "cursor":
        this.#patch({ cursor: cursorStyle(event, this.#o.toDataUrl) });
        break;
      case "certPrompt":
        this.#patch({
          cert: { fingerprint: event.fingerprint, changed: event.changed, stored: event.stored },
        });
        break;
      case "unencryptedPrompt":
        this.#patch({ unencryptedPrompt: true });
        break;
      case "credentialPrompt": {
        const { type: _type, ...prompt } = event;
        this.#patch({ credential: prompt });
        break;
      }
      case "security":
        this.#patch({ transport: event.encrypted ? "encrypted" : "clear", auth: event.auth ?? null });
        break;
      case "clipboard":
        this.#patch({ clipboard: reduceClipboard(this.#state.clipboard, event.event) });
        break;
      case "closed":
        this.renderer.flush();
        this.input.releaseAll();
        this.#patch({
          phase: "closed",
          closedReason: event.reason,
          cert: null,
          unencryptedPrompt: false,
          credential: null,
        });
        break;
      case "error":
        this.renderer.flush();
        if (event.error.code === "cancelled") {
          // The user's own choice: a calm "cancelled" state, no error handling.
          this.#saveRequested = false;
          this.#patch({ phase: "cancelled", cert: null, unencryptedPrompt: false, credential: null });
        } else if (event.error.code === "security_downgrade") {
          // Not a generic failure: the panel explains it and offers the deliberate way forward.
          this.input.releaseAll();
          this.#patch({ downgrade: event.error.message, unencryptedPrompt: false, credential: null });
        } else {
          this.#o.onFatal(event.error);
        }
        break;
    }
  }

  /** The tab is shown or hidden. Hiding releases held keys; showing asks for nothing more:
   *  the backend sends a full redraw by itself. */
  setVisible(visible: boolean): void {
    if (this.#disposed || this.#visible === visible) return;
    this.#visible = visible;
    if (!visible) this.input.releaseAll();
    this.#o.api.setVisible(visible).catch((e: unknown) => this.#o.onError("fb_set_visible", e));
  }

  /** The drawing area changed size: ask the remote side for the same size, debounced. The
   *  backend ignores it when it cannot resize; the view then just scales the image. */
  requestResize(width: number, height: number): void {
    if (this.#disposed) return;
    const w = Math.floor(width);
    const h = Math.floor(height);
    if (w <= 0 || h <= 0) return;
    const timer = this.#o.setTimer ?? ((cb, ms) => window.setTimeout(cb, ms));
    const clear = this.#o.clearTimer ?? ((handle) => window.clearTimeout(handle));
    if (this.#resizeTimer !== null) clear(this.#resizeTimer);
    this.#resizeTimer = timer(() => {
      this.#resizeTimer = null;
      const remote = this.#state.remote;
      if (this.#state.phase !== "live" || (remote.w === w && remote.h === h)) return;
      if (this.#lastRequested?.w === w && this.#lastRequested.h === h) return;
      this.#lastRequested = { w, h };
      this.#o.api.resize(w, h).catch((e: unknown) => this.#o.onError("fb_resize", e));
    }, this.#o.resizeDebounceMs ?? 300);
  }

  /** The user answered the prompt for an unknown certificate. (A changed certificate has
   *  nothing to answer: see `forgetCertificateAndReconnect`.) */
  decideCert(accept: boolean): void {
    if (!this.#state.cert || this.#state.cert.changed) return;
    this.#patch({ cert: null });
    this.#o.api.certDecision(accept).catch((e: unknown) => this.#o.onError("fb_cert_decision", e));
  }

  /** The user's answer to "this server offers no encryption": `true` sends the password anyway
   *  (and remembers the host), `false` ends the session. */
  decideUnencrypted(allow: boolean): void {
    if (!this.#state.unencryptedPrompt) return;
    this.#patch({ unencryptedPrompt: false });
    this.#o.api
      .unencryptedDecision(allow)
      .catch((e: unknown) => this.#o.onError("fb_unencrypted_decision", e));
  }

  /**
   * The deliberate way forward after a changed certificate or a refused downgrade: remove the
   * stored certificate of the host, then connect again (where it is a new certificate that the
   * user must confirm like the first time).
   */
  forgetCertificateAndReconnect(): void {
    if (!this.#state.cert?.changed && this.#state.downgrade === null) return;
    this.#o.api.forgetCertificate().then(
      () => this.#o.onReconnect?.(),
      (e: unknown) => this.#o.onError("fb_forget_certificate", e),
    );
  }

  /**
   * The user's answer to the credential prompt. The secrets are handed to the backend and
   * dropped here: nothing of the answer is kept.
   */
  submitCredential(answer: CredentialAnswer, saveToVault: boolean): void {
    if (!this.#state.credential) return;
    this.#saveRequested = saveToVault;
    this.#patch({ credential: null });
    this.#o.api.credential(answer, saveToVault).catch((e: unknown) => {
      // The message of an IPC error never carries the secret.
      this.#o.onError("session_credential", e);
      this.#o.onFatal(
        e instanceof Error && "code" in e
          ? { code: (e as { code: AppError["code"] }).code, message: e.message }
          : { code: "internal", message: "The credentials could not be sent." },
      );
    });
  }

  /**
   * The user asked to save the files on the remote clipboard. The backend opens the folder
   * dialog; nothing is saved when it is dismissed.
   */
  saveClipboardFiles(): void {
    const { clipboard } = this.#state;
    if (clipboard.offer === null || clipboard.phase.kind === "saving") return;
    this.#o.api.clipboardSave().then(
      (started) => {
        if (started) this.#patch({ clipboard: startSaving(this.#state.clipboard) });
      },
      (e: unknown) => {
        this.#o.onError("fb_clipboard_save", e);
        this.#patch({ clipboard: failedLocally(this.#state.clipboard) });
      },
    );
  }

  /** The user stopped the save. The outcome arrives as an event. */
  cancelClipboardSave(): void {
    if (this.#state.clipboard.phase.kind !== "saving") return;
    this.#o.api.clipboardCancel().catch((e: unknown) => this.#o.onError("fb_clipboard_cancel", e));
  }

  /** The user closed the notice about a finished or failed save. */
  dismissClipboardNotice(): void {
    this.#patch({ clipboard: dismissNotice(this.#state.clipboard) });
  }

  /** The user dismissed the credential prompt: the session ends as cancelled. */
  cancelCredential(): void {
    if (!this.#state.credential) return;
    this.#saveRequested = false;
    this.#patch({ credential: null, phase: "cancelled" });
    this.#o.api.credentialCancel().catch((e: unknown) => this.#o.onError("session_credential_cancel", e));
  }

  dispose(): void {
    if (this.#disposed) return;
    this.input.releaseAll();
    this.#disposed = true;
    if (this.#resizeTimer !== null) {
      (this.#o.clearTimer ?? ((handle) => window.clearTimeout(handle)))(this.#resizeTimer);
    }
    this.input.dispose();
    this.renderer.dispose();
  }
}
