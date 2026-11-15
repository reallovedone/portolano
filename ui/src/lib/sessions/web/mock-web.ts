// Fake web sessions for the mock IPC: no webview exists, but the commands behave like the
// backend's (scheme validation, unknown session, navigation history) so the UI can be reviewed
// and tested in a plain browser.

import { IpcError } from "../../ipc/errors";
import type { Bounds, Document, Uuid, WebNavAction } from "../../ipc/types";
import { normalizeUrl } from "./url";

interface MockWebSession {
  connectionId: Uuid;
  bounds: Bounds;
  visible: boolean;
  history: string[];
  index: number;
}

export interface MockWeb {
  open(connectionId: Uuid, bounds: Bounds): Promise<string>;
  setBounds(sessionId: string, bounds: Bounds, visible: boolean): Promise<void>;
  navigate(sessionId: string, action: WebNavAction): Promise<void>;
  /** `true` when `sessionId` was a mock web session (now closed). */
  close(sessionId: string): boolean;
  /** Current state of a session (tests and the stand-in panel). */
  inspect(sessionId: string): { url: string; bounds: Bounds; visible: boolean } | null;
}

function rejectUnsupported(what: string): Promise<never> {
  return Promise.reject(new IpcError("unsupported", `${what} is not available in the mock IPC`));
}

export function createMockWeb(getDoc: () => Document, enabled: boolean): MockWeb {
  if (!enabled) {
    return {
      open: () => rejectUnsupported("webOpen"),
      setBounds: () => rejectUnsupported("webSetBounds"),
      navigate: () => rejectUnsupported("webNavigate"),
      close: () => false,
      inspect: () => null,
    };
  }

  const sessions = new Map<string, MockWebSession>();
  let counter = 0;

  const session = (sessionId: string): MockWebSession => {
    const s = sessions.get(sessionId);
    if (!s) throw new IpcError("not_found", `session ${sessionId} not found`);
    return s;
  };

  return {
    async open(connectionId, bounds) {
      const node = getDoc().nodes.find((n) => n.id === connectionId);
      if (!node) throw new IpcError("not_found", `connection ${connectionId} not found`);
      if (node.type !== "connection" || node.options.type !== "web") {
        throw new IpcError("invalid_input", `connection ${connectionId} is not a web connection`);
      }
      const checked = normalizeUrl(node.options.url);
      if (!checked.ok || !/^https?:\/\//i.test(node.options.url.trim())) {
        throw new IpcError("invalid_input", "invalid URL: only http and https are allowed");
      }
      counter += 1;
      const sessionId = `mock-web-${counter}`;
      sessions.set(sessionId, {
        connectionId,
        bounds,
        visible: false,
        history: [checked.url],
        index: 0,
      });
      return sessionId;
    },
    async setBounds(sessionId, bounds, visible) {
      const s = session(sessionId);
      if (![bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite)) {
        throw new IpcError("invalid_input", "bounds must be finite numbers");
      }
      s.bounds = bounds;
      s.visible = visible;
    },
    async navigate(sessionId, action) {
      const s = session(sessionId);
      switch (action.type) {
        case "back":
          s.index = Math.max(0, s.index - 1);
          return;
        case "forward":
          s.index = Math.min(s.history.length - 1, s.index + 1);
          return;
        case "reload":
          return;
        case "url": {
          const checked = normalizeUrl(action.url);
          if (!checked.ok || !/^https?:\/\//i.test(action.url.trim())) {
            throw new IpcError("invalid_input", "invalid URL: only http and https are allowed");
          }
          s.history = [...s.history.slice(0, s.index + 1), checked.url];
          s.index = s.history.length - 1;
          return;
        }
      }
    },
    close(sessionId) {
      return sessions.delete(sessionId);
    },
    inspect(sessionId) {
      const s = sessions.get(sessionId);
      return s ? { url: s.history[s.index] ?? "", bounds: s.bounds, visible: s.visible } : null;
    },
  };
}
