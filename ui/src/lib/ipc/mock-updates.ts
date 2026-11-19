// Updates of the mock IPC. Nothing is downloaded or installed: the answers are canned and tests
// (or `?ipc=mock` demos) steer them through `MockOptions.updates` and the returned controller.

import { IpcError } from "./errors";
import type { UpdateCheck, UpdateEvent, UpdateInfo, UpdateSettings, WhatsNew } from "./types";

export interface MockUpdateOptions {
  /** Version of the "running" application (default 0.1.0). */
  currentVersion?: string;
  /** Initial "Automatically check for updates" (default true). */
  autoCheck?: boolean;
  /** An update the server "offers" (default: none, the application is up to date). */
  available?: Pick<UpdateInfo, "version">;
  /** Make the check fail with this message (a network error). */
  checkError?: string;
  /** Make the installation fail with this message. */
  installError?: string;
  /** The "What's new" due at start-up (default: none). */
  whatsNew?: WhatsNew;
}

/** What tests look at, and the knobs they turn while the UI is running. */
export interface MockUpdates {
  /** Every `updateCheck` call, in order (the `manual` flag). */
  readonly checks: boolean[];
  /** Versions passed to `updateSkip`. */
  readonly skipped: string[];
  /** Versions whose release page was opened. */
  readonly openedPages: string[];
  /** Number of times the full changelog was opened. */
  openedChangelogs: number;
  /** Number of `updateInstall` calls. */
  installs: number;
  /** Current values; tests may change them while the UI runs. */
  options: MockUpdateOptions;
  settings: UpdateSettings;
  /** Resolves the progress of a running `updateInstall` (tests that look at the progress bar). */
  holdInstall: boolean;
  releaseInstall(): void;
  readonly api: {
    updateSettings(): Promise<UpdateSettings>;
    updateSetAutoCheck(enabled: boolean): Promise<void>;
    updateCheck(manual: boolean): Promise<UpdateCheck>;
    updateSkip(version: string): Promise<void>;
    updateInstall(onEvent: (event: UpdateEvent) => void): Promise<void>;
    whatsNew(): Promise<WhatsNew | null>;
    updateOpenReleasePage(version: string): Promise<void>;
    updateOpenChangelog(): Promise<void>;
  };
}

export function createMockUpdates(options: MockUpdateOptions = {}): MockUpdates {
  let release: (() => void) | null = null;
  const state: MockUpdates = {
    checks: [],
    skipped: [],
    openedPages: [],
    openedChangelogs: 0,
    installs: 0,
    options,
    settings: {
      autoCheck: options.autoCheck ?? true,
      currentVersion: options.currentVersion ?? "0.1.0",
    },
    holdInstall: false,
    releaseInstall: () => release?.(),
    api: {
      updateSettings: async () => ({ ...state.settings }),
      updateSetAutoCheck: async (enabled) => {
        state.settings = { ...state.settings, autoCheck: enabled };
      },
      updateCheck: async (manual) => {
        state.checks.push(manual);
        const { checkError, available } = state.options;
        if (!manual && !state.settings.autoCheck) return { status: "disabled" };
        if (checkError !== undefined) throw new IpcError("host_unreachable", checkError);
        if (!available) return { status: "upToDate" };
        if (!manual && state.skipped.includes(available.version)) {
          return { status: "skipped", version: available.version };
        }
        return {
          status: "available",
          update: { currentVersion: state.settings.currentVersion, ...available },
        };
      },
      updateSkip: async (version) => {
        state.skipped.push(version);
      },
      updateInstall: async (onEvent) => {
        state.installs += 1;
        onEvent({ type: "progress", downloaded: 512, total: 2048 });
        if (state.holdInstall) {
          await new Promise<void>((resolve) => {
            release = resolve;
          });
        }
        if (state.options.installError !== undefined) {
          throw new IpcError("internal", state.options.installError);
        }
        onEvent({ type: "installing" });
      },
      whatsNew: async () => state.options.whatsNew ?? null,
      updateOpenReleasePage: async (version) => {
        state.openedPages.push(version);
      },
      updateOpenChangelog: async () => {
        state.openedChangelogs += 1;
      },
    },
  };
  return state;
}
