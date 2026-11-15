// Test support: a reactive `SessionContext` for mounting `FramebufferView` outside the shell.

import { vi } from "vitest";

import type { AppError } from "../../ipc";
import { SessionBridge, type SessionContext } from "../../tabs/session";

export function createTestSession(sessionId = "sess-1") {
  const state = $state({ visible: true, overlayOpen: false });
  const fail = vi.fn<(error: AppError) => void>();
  const retry = vi.fn<() => void>();
  const bridge = new SessionBridge();
  const context: SessionContext = {
    tabId: "tab-1",
    connectionId: "conn-1",
    kind: "rdp",
    sessionId,
    bridge,
    get visible() {
      return state.visible;
    },
    get overlayOpen() {
      return state.overlayOpen;
    },
    get shouldShow() {
      return state.visible && !state.overlayOpen;
    },
    fail,
    retry,
  };
  return { context, bridge, fail, retry, state };
}
