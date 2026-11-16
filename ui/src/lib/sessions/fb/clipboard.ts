// What the session tab shows about the shared clipboard (RDP). The backend sends counts and
// sizes only (`clipboard` events); file names and clipboard contents never reach the UI.
// Pure functions: the controller keeps the state, the view renders it.

import type { ClipboardEvent, ClipboardFailure } from "../../ipc";

/** `unavailable` is raised by the UI itself when the command was refused. */
export type ClipboardFailureView = ClipboardFailure | "unavailable";

export interface ClipboardOffer {
  /** Files and folders on the remote clipboard. */
  count: number;
  bytes: number;
  /** Entries left out because their names could not be made safe. */
  skipped: number;
}

export interface ClipboardProgress {
  filesDone: number;
  filesTotal: number;
  bytesDone: number;
  bytesTotal: number;
}

export interface ClipboardUi {
  /** Files that can be saved now; kept while a notice about a save is shown. */
  offer: ClipboardOffer | null;
  phase:
    | { kind: "idle" }
    | { kind: "saving"; progress: ClipboardProgress }
    | { kind: "done"; files: number; bytes: number; skipped: number }
    | { kind: "failed"; reason: ClipboardFailureView };
}

export const CLIPBOARD_IDLE: ClipboardUi = { offer: null, phase: { kind: "idle" } };

/** The state after the backend reported `event`. */
export function reduceClipboard(state: ClipboardUi, event: ClipboardEvent): ClipboardUi {
  const saving = state.phase.kind === "saving";
  switch (event.type) {
    case "remoteFiles":
      // A save in progress goes on whatever the remote machine copies meanwhile.
      if (saving) return state;
      return {
        offer:
          event.count > 0
            ? { count: event.count, bytes: event.bytes, skipped: event.skipped }
            : null,
        phase: { kind: "idle" },
      };
    case "progress":
      return {
        ...state,
        phase: {
          kind: "saving",
          progress: {
            filesDone: event.filesDone,
            filesTotal: event.filesTotal,
            bytesDone: event.bytesDone,
            bytesTotal: event.bytesTotal,
          },
        },
      };
    case "done":
      return {
        ...state,
        phase: { kind: "done", files: event.files, bytes: event.bytes, skipped: event.skipped },
      };
    case "failed":
      // `nothing` and `limit` can also mean that no offer exists (any more).
      return {
        offer: event.reason === "nothing" || event.reason === "limit" ? null : state.offer,
        phase: { kind: "failed", reason: event.reason },
      };
  }
}

/** The save was started (the folder was chosen). */
export function startSaving(state: ClipboardUi): ClipboardUi {
  const total = state.offer?.count ?? 0;
  return {
    ...state,
    phase: {
      kind: "saving",
      progress: {
        filesDone: 0,
        filesTotal: total,
        bytesDone: 0,
        bytesTotal: state.offer?.bytes ?? 0,
      },
    },
  };
}

/** The user dismissed the notice about a finished or failed save. */
export function dismissNotice(state: ClipboardUi): ClipboardUi {
  const kind = state.phase.kind;
  if (kind === "done" || kind === "failed") return { ...state, phase: { kind: "idle" } };
  return state;
}

/** A failed command (the backend refused or the IPC failed). */
export function failedLocally(state: ClipboardUi): ClipboardUi {
  return { ...state, phase: { kind: "failed", reason: "unavailable" } };
}

const UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** `1536` -> "1.5 KB". Decimal units, one decimal for KB and up. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "0 B";
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  const shown = unit === 0 ? String(Math.round(value)) : value.toFixed(value < 10 ? 1 : 0);
  return `${shown} ${UNITS[unit]}`;
}

/** 0..100, never NaN. */
export function percent(done: number, total: number): number {
  if (total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((done / total) * 100)));
}
