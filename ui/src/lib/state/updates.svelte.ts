// Updates in the UI: the start-up sequence ("What's new" first, then the update check), the
// daily check, and the state of the update offer and installation. All networking happens in the
// backend; this store only calls its commands.

import { t } from "../i18n/strings";
import {
  toIpcError,
  type Ipc,
  type IpcError,
  type UpdateCheck,
  type UpdateInfo,
  type UpdateSettings,
  type WhatsNew,
} from "../ipc";
import type { NoticeStore } from "./notices.svelte";

export interface UpdateOffer {
  info: UpdateInfo;
  /** From "Check for updates…": shown at once, even over another dialog. */
  manual: boolean;
}

/**
 * What the status bar shows: "upToDate" after a check found nothing newer, "available" when a
 * newer version exists (also when the user skipped it), "unknown" before any answer or while
 * automatic checks are off.
 */
export type Freshness = "unknown" | "upToDate" | "available";

export type InstallState =
  | { phase: "downloading"; downloaded: number; total: number | null }
  | { phase: "installing" }
  | { phase: "restarting" }
  | { phase: "failed"; error: IpcError };

export interface UpdateTiming {
  /** Delay after start-up before the first automatic check. */
  startDelayMs: number;
  /** Interval between automatic checks while the application keeps running. */
  intervalMs: number;
}

export const DEFAULT_TIMING: UpdateTiming = {
  startDelayMs: 8_000,
  intervalMs: 24 * 60 * 60 * 1000,
};

export class UpdateStore {
  settings = $state<UpdateSettings | null>(null);
  /** The "What's new" dialog due at start-up. */
  whatsNew = $state<WhatsNew | null>(null);
  /** An update waiting for the user's decision. */
  offer = $state<UpdateOffer | null>(null);
  install = $state<InstallState | null>(null);
  freshness = $state<Freshness>("unknown");

  readonly #ipc: Ipc;
  readonly #notices: NoticeStore;
  readonly #timing: UpdateTiming;

  constructor(ipc: Ipc, notices: NoticeStore, timing: UpdateTiming = DEFAULT_TIMING) {
    this.#ipc = ipc;
    this.#notices = notices;
    this.#timing = timing;
  }

  /** Automatic checks on? On until the backend says otherwise (it enforces the setting too). */
  get autoCheck(): boolean {
    return this.settings?.autoCheck ?? true;
  }

  get installing(): boolean {
    const phase = this.install?.phase;
    return phase === "downloading" || phase === "installing" || phase === "restarting";
  }

  /**
   * Start-up sequence: the settings, then "What's new" (a failure here is only logged, never an
   * error dialog), and after a short delay the first automatic check, then one every interval.
   * Returns the function that stops the timers.
   */
  start(): () => void {
    let stopped = false;
    let first: ReturnType<typeof setTimeout> | undefined;
    let repeat: ReturnType<typeof setInterval> | undefined;

    void (async () => {
      await this.loadSettings();
      await this.loadWhatsNew();
      if (stopped) return;
      first = setTimeout(() => {
        void this.checkInBackground();
        repeat = setInterval(() => void this.checkInBackground(), this.#timing.intervalMs);
      }, this.#timing.startDelayMs);
    })();

    return () => {
      stopped = true;
      clearTimeout(first);
      clearInterval(repeat);
    };
  }

  async loadSettings(): Promise<void> {
    try {
      this.settings = await this.#ipc.updateSettings();
    } catch (e) {
      this.#log("warn", `update settings unavailable: ${toIpcError(e).message}`);
    }
  }

  async loadWhatsNew(): Promise<void> {
    try {
      this.whatsNew = await this.#ipc.whatsNew();
    } catch (e) {
      // Nothing is shown at start-up for this: it is a courtesy, not an error.
      this.#log("warn", `what's new unavailable: ${toIpcError(e).message}`);
    }
  }

  dismissWhatsNew(): void {
    this.whatsNew = null;
  }

  /** Turns the automatic checks on or off. Resolves `false` (and tells the user) on failure. */
  async setAutoCheck(enabled: boolean): Promise<boolean> {
    try {
      await this.#ipc.updateSetAutoCheck(enabled);
      this.settings = {
        autoCheck: enabled,
        currentVersion: this.settings?.currentVersion ?? "",
      };
      if (!enabled) this.freshness = "unknown";
      return true;
    } catch (e) {
      this.#notices.error(t("update.autoFailed"), e);
      return false;
    }
  }

  /** A scheduled check: silent unless an update is found (errors are in the backend log). */
  async checkInBackground(): Promise<void> {
    if (!this.autoCheck || this.offer || this.installing) return;
    try {
      const result = await this.#ipc.updateCheck(false);
      this.#recordFreshness(result);
      if (result.status === "available" && !this.offer && !this.installing) {
        this.offer = { info: result.update, manual: false };
      }
    } catch (e) {
      this.#log("warn", `automatic update check failed: ${toIpcError(e).message}`);
    }
  }

  /** "Check for updates…": always answers; rejects with the `IpcError` on failure. */
  async checkNow(): Promise<UpdateCheck> {
    try {
      const result = await this.#ipc.updateCheck(true);
      this.#recordFreshness(result);
      if (result.status === "available") this.offer = { info: result.update, manual: true };
      return result;
    } catch (e) {
      throw toIpcError(e);
    }
  }

  /** "Later": nothing is remembered; the next scheduled check proposes it again. */
  later(): void {
    if (!this.installing) this.offer = null;
  }

  /** "Skip this version": remembered; only a manual check shows it again. */
  async skip(): Promise<boolean> {
    const offer = this.offer;
    if (!offer || this.installing) return false;
    try {
      await this.#ipc.updateSkip(offer.info.version);
      this.offer = null;
      return true;
    } catch (e) {
      this.#notices.error(t("update.skipFailed"), e);
      return false;
    }
  }

  /** "Update now". On success the application restarts; failures stay on screen. */
  async installNow(): Promise<void> {
    if (!this.offer || this.installing) return;
    this.install = { phase: "downloading", downloaded: 0, total: null };
    try {
      await this.#ipc.updateInstall((event) => {
        if (event.type === "progress") {
          this.install = { phase: "downloading", downloaded: event.downloaded, total: event.total };
        } else {
          this.install = { phase: "installing" };
        }
      });
      this.install = { phase: "restarting" };
    } catch (e) {
      this.install = { phase: "failed", error: toIpcError(e) };
    }
  }

  /** Back from a failed installation to the offer (to try again or to choose something else). */
  clearFailure(): void {
    if (this.install?.phase === "failed") this.install = null;
  }

  async openReleasePage(version: string): Promise<void> {
    try {
      await this.#ipc.updateOpenReleasePage(version);
    } catch (e) {
      this.#notices.error(t("update.openNotesFailed"), e);
    }
  }

  #recordFreshness(result: UpdateCheck): void {
    switch (result.status) {
      case "upToDate":
        this.freshness = "upToDate";
        break;
      case "available":
      case "skipped":
        this.freshness = "available";
        break;
      case "disabled":
        this.freshness = "unknown";
        break;
    }
  }

  #log(level: "info" | "warn", message: string): void {
    // A log line that cannot be written is not worth telling the user about.
    this.#ipc.uiLog(level, message).catch(() => undefined);
  }
}
