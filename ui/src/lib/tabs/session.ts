// Contract between the shell and the session components (WP5-WP8).
//
// The shell opens the backend session itself (so a failure can be shown in the tab with a
// retry action) and renders the session component with `sessionId`. The component gets
// everything else from `getSessionContext()`:
//   - `bridge`: the Channel events of the session (TermEvent / FbEvent). Events that arrive
//     before the component subscribes are buffered and replayed to the first subscriber.
//   - `visible` / `overlayOpen` / `shouldShow`: reactive getters. `visible` is "this tab is
//     the active one". `overlayOpen` is true while a menu, dialog or unlock screen is shown.
//     `shouldShow` = visible && !overlayOpen: what a child webview must follow.
//   - `fail(error)` / `retry()`: report a fatal error after open, or re-open the session.

import { getContext, setContext } from "svelte";

import type {
  AppError,
  FbEvent,
  Ipc,
  OpenOptions,
  SessionId,
  TermEvent,
  Uuid,
  Connection,
} from "../ipc";
import type { ConnectionKind } from "../tree/model";
import type { SessionToolbarModel } from "./toolbar.svelte";

export type SessionEvent = TermEvent | FbEvent;

export interface SessionGeometry {
  /** Size of the tab content area in CSS pixels. */
  width: number;
  height: number;
  /** Position of the content area in the window (for child webviews). */
  x: number;
  y: number;
}

export const DEFAULT_GEOMETRY: SessionGeometry = { width: 1024, height: 768, x: 0, y: 0 };

// Approximate terminal cell size used for the initial pty size; the terminal view refines it
// with `term_resize` once it has measured its font.
const CELL_WIDTH = 8;
const CELL_HEIGHT = 17;

/** Buffers events until a consumer subscribes, then forwards them live. */
export class SessionBridge {
  #buffer: SessionEvent[] = [];
  #handler: ((event: SessionEvent) => void) | null = null;
  readonly #observer: ((event: SessionEvent) => void) | undefined;

  /** `observer` sees every event as it arrives, whether or not a consumer is subscribed. */
  constructor(observer?: (event: SessionEvent) => void) {
    this.#observer = observer;
  }

  emit(event: SessionEvent): void {
    this.#observer?.(event);
    if (this.#handler) this.#handler(event);
    else this.#buffer.push(event);
  }

  /** Subscribes (one consumer at a time); returns the unsubscribe function. */
  subscribe(handler: (event: SessionEvent) => void): () => void {
    this.#handler = handler;
    const pending = this.#buffer;
    this.#buffer = [];
    for (const event of pending) handler(event);
    return () => {
      if (this.#handler === handler) this.#handler = null;
    };
  }
}

export interface SessionContext {
  readonly tabId: string;
  readonly connectionId: Uuid;
  readonly kind: ConnectionKind;
  readonly sessionId: SessionId;
  readonly bridge: SessionBridge;
  readonly visible: boolean;
  readonly overlayOpen: boolean;
  readonly shouldShow: boolean;
  fail(error: AppError): void;
  retry(): void;
  /**
   * Hands the session toolbar what this view knows (target, resolution, protection, extra
   * actions). Returns the function that withdraws it; call it when the view goes away.
   */
  publishToolbar(model: SessionToolbarModel): () => void;
}

const SESSION_KEY = Symbol("portolano.session");

export function provideSessionContext(context: SessionContext): void {
  setContext(SESSION_KEY, context);
}

export function getSessionContext(): SessionContext {
  const context = getContext<SessionContext | undefined>(SESSION_KEY);
  if (!context) throw new Error("getSessionContext() called outside a session tab");
  return context;
}

/** Opens the backend session for a connection, routing events into `bridge`. */
export function openSession(
  ipc: Ipc,
  connection: Connection,
  bridge: SessionBridge,
  geometry: SessionGeometry = DEFAULT_GEOMETRY,
  options: OpenOptions = {},
): Promise<SessionId> {
  const forward = (event: SessionEvent): void => bridge.emit(event);
  switch (connection.options.type) {
    case "localShell":
    case "ssh": {
      const cols = Math.max(20, Math.floor(geometry.width / CELL_WIDTH));
      const rows = Math.max(5, Math.floor(geometry.height / CELL_HEIGHT));
      return ipc.termOpen(connection.id, cols, rows, forward, options);
    }
    case "rdp":
    case "vnc":
      return ipc.fbOpen(
        connection.id,
        Math.max(1, Math.round(geometry.width)),
        Math.max(1, Math.round(geometry.height)),
        forward,
        options,
      );
    case "web":
      return ipc.webOpen(connection.id, {
        x: geometry.x,
        y: geometry.y,
        width: geometry.width,
        height: geometry.height,
      });
  }
}
