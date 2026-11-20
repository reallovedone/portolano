// Test support: a reactive `SessionContext` for mounting `FramebufferView` outside the shell.

import { vi } from "vitest";

import type { AppError } from "../../ipc";
import type { App } from "../../state/app";
import { SessionBridge, type SessionContext } from "../../tabs/session";
import type { SessionToolbarModel } from "../../tabs/toolbar.svelte";

export function createTestSession(sessionId = "sess-1", options: { app?: App } = {}) {
  const state = $state({ visible: true, overlayOpen: false });
  const fail = vi.fn<(error: AppError) => void>();
  const retry = vi.fn<() => void>();
  const bridge = new SessionBridge();
  /** The model the view published, as the toolbar would read it. */
  const published = $state<{ model: SessionToolbarModel | null }>({ model: null });
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
    publishToolbar: (model) => {
      published.model = model;
      // Also in the app's store when one is attached (so a mounted toolbar can read it).
      const withdraw = options.app?.toolbar.publish(context.tabId, model);
      return () => {
        withdraw?.();
        if (published.model === model) published.model = null;
      };
    },
  };
  return { context, bridge, fail, retry, state, published };
}
