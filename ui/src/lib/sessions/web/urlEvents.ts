// Live address-bar updates: the backend emits `web-url-changed` when the page of a session
// starts or finishes loading. Only the real app has them; the mock IPC never emits.

import type { SessionId } from "../../ipc";

export interface UrlChange {
  sessionId: SessionId;
  url: string;
  /** "started" while the page loads, "finished" once it did. */
  phase: "started" | "finished";
}

export type UrlSubscribe = (handler: (change: UrlChange) => void) => () => void;

const EVENT = "web-url-changed";

/** Subscribes through the Tauri event API (loaded lazily: it does not exist in a browser). */
export const tauriUrlEvents: UrlSubscribe = (handler) => {
  let unlisten: (() => void) | null = null;
  let stopped = false;
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen<{ sessionId: SessionId; url: string; phase: UrlChange["phase"] }>(EVENT, (event) =>
        handler(event.payload),
      ),
    )
    .then((off) => {
      if (stopped) off();
      else unlisten = off;
    })
    // Without the listener the address bar just keeps the last URL it knows.
    .catch(() => undefined);
  return () => {
    stopped = true;
    unlisten?.();
  };
};

export const noUrlEvents: UrlSubscribe = () => () => undefined;
