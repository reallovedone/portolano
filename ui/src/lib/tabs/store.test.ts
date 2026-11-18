import { describe, expect, it, vi } from "vitest";

import { IpcError, type Ipc } from "../ipc";
import { MOCK_IDS, MOCK_MASTER_PASSWORD, createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";

/** A mock whose session commands succeed, recording opens and closes. */
function workingIpc(base: Ipc) {
  const opened: string[] = [];
  const closed: string[] = [];
  let counter = 0;
  const ipc: Ipc = {
    ...base,
    termOpen: async (id, _cols, _rows, _onEvent, options) => {
      opened.push(`term:${id}${options?.askCredentials ? ":ask" : ""}`);
      return `s${++counter}`;
    },
    fbOpen: async (id, _w, _h, _onEvent, options) => {
      opened.push(`fb:${id}${options?.askCredentials ? ":ask" : ""}`);
      return `s${++counter}`;
    },
    webOpen: async (id) => {
      opened.push(`web:${id}`);
      return `s${++counter}`;
    },
    sessionClose: async (sid) => {
      closed.push(sid);
    },
  };
  return { ipc, opened, closed };
}

async function ready(ipc: Ipc) {
  const app = createApp(ipc);
  await Promise.all([app.doc.load(), app.vault.refresh()]);
  return app;
}

describe("TabsStore", () => {
  it("routes each kind to its session family", async () => {
    const { ipc, opened } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    await app.tabs.openConnection(MOCK_IDS.localSshd); // ssh
    await app.tabs.openConnection(MOCK_IDS.devVm); // vnc
    await app.tabs.openConnection(MOCK_IDS.rdpServer); // rdp
    await app.tabs.openConnection(MOCK_IDS.intranet); // web
    await app.tabs.openConnection(MOCK_IDS.localZsh); // local shell
    expect(opened.map((o) => o.split(":")[0])).toEqual(["term", "fb", "fb", "web", "term"]);
    expect(app.tabs.tabs.every((t) => t.status === "open" && t.sessionId !== null)).toBe(true);
    expect(app.tabs.state.activeId).toBe(app.tabs.tabs[app.tabs.tabs.length - 1]?.id);
  });

  it("shows the unsupported error of the mock on the tab and keeps the tab open", async () => {
    const app = await ready(createMockIpc({ vault: "unlocked" }));
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    const tab = app.tabs.activeTab;
    expect(tab?.status).toBe("failed");
    expect(tab?.error?.code).toBe("unsupported");
    expect(app.tabs.tabs).toHaveLength(1);
  });

  it("prompts for the unlock screen first when the credential needs the vault", async () => {
    const { ipc, opened } = workingIpc(createMockIpc({ vault: "locked" }));
    const app = await ready(ipc);

    const opening = app.tabs.openConnection(MOCK_IDS.localSshd); // inherits "Lab admin"
    await vi.waitFor(() => expect(app.vault.request).not.toBeNull());
    expect(app.vault.request?.reason).toBe("Local sshd");
    expect(app.tabs.tabs).toHaveLength(0);

    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    app.vault.finishRequest(true);
    expect(await opening).not.toBeNull();
    expect(opened).toHaveLength(1);
  });

  it("opens nothing when the unlock screen is cancelled", async () => {
    const { ipc, opened } = workingIpc(createMockIpc({ vault: "locked" }));
    const app = await ready(ipc);
    const opening = app.tabs.openConnection(MOCK_IDS.localSshd);
    await vi.waitFor(() => expect(app.vault.request).not.toBeNull());
    app.vault.finishRequest(false);
    expect(await opening).toBeNull();
    expect(app.tabs.tabs).toHaveLength(0);
    expect(opened).toHaveLength(0);
  });

  it("does not need the vault for connections without a credential", async () => {
    const { ipc } = workingIpc(createMockIpc({ vault: "locked" }));
    const app = await ready(ipc);
    expect(await app.tabs.openConnection(MOCK_IDS.localZsh)).not.toBeNull();
    expect(app.vault.request).toBeNull();
  });

  it("retries a failed tab and replaces the error", async () => {
    let fail = true;
    const { ipc } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const flaky: Ipc = {
      ...ipc,
      termOpen: async (id, c, r, cb) => {
        if (fail) throw new IpcError("host_unreachable", "no route");
        return ipc.termOpen(id, c, r, cb);
      },
    };
    const app = await ready(flaky);
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    expect(app.tabs.activeTab?.error).toEqual({ code: "host_unreachable", message: "no route" });

    fail = false;
    const id = app.tabs.activeTab?.id ?? "";
    await app.tabs.retry(id);
    expect(app.tabs.activeTab).toMatchObject({ status: "open", error: null, attempt: 1 });
  });

  it("unlock-and-retry unlocks first when the backend says vault_locked", async () => {
    const base = createMockIpc({ vault: "unlocked" });
    let locked = true;
    const { ipc } = workingIpc(base);
    const gated: Ipc = {
      ...ipc,
      termOpen: async (id, c, r, cb) => {
        if (locked) throw new IpcError("vault_locked", "the vault is locked");
        return ipc.termOpen(id, c, r, cb);
      },
    };
    const app = await ready(gated);
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    expect(app.tabs.activeTab?.error?.code).toBe("vault_locked");

    await app.vault.lock();
    locked = false;
    const id = app.tabs.activeTab?.id ?? "";
    const retrying = app.tabs.unlockAndRetry(id);
    await vi.waitFor(() => expect(app.vault.request).not.toBeNull());
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    app.vault.finishRequest(true);
    await retrying;
    expect(app.tabs.activeTab?.status).toBe("open");
  });

  it("closes backend sessions when tabs close, for one, others, all and deleted connections", async () => {
    const { ipc, closed } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const a = (await app.tabs.openConnection(MOCK_IDS.localZsh)) ?? "";
    const b = (await app.tabs.openConnection(MOCK_IDS.devVm)) ?? "";
    await app.tabs.openConnection(MOCK_IDS.intranet);
    await app.tabs.openConnection(MOCK_IDS.rdpServer);
    expect(app.tabs.tabs).toHaveLength(4);

    await app.tabs.close(a);
    expect(closed).toEqual(["s1"]);

    await app.tabs.closeForConnections(new Set([MOCK_IDS.intranet]));
    expect(closed).toEqual(["s1", "s3"]);
    expect(app.tabs.tabs).toHaveLength(2);

    await app.tabs.closeOthers(b);
    expect(app.tabs.tabs.map((t) => t.id)).toEqual([b]);
    expect(closed).toEqual(["s1", "s3", "s4"]);

    await app.tabs.closeAll();
    expect(app.tabs.tabs).toHaveLength(0);
    expect(closed).toEqual(["s1", "s3", "s4", "s2"]);
  });

  it("does not leak a session that finishes opening after its tab was closed", async () => {
    const closed: string[] = [];
    let release: (id: string) => void = () => undefined;
    const slow: Ipc = {
      ...createMockIpc({ vault: "unlocked" }),
      termOpen: () =>
        new Promise<string>((resolve) => {
          release = resolve;
        }),
      sessionClose: async (sid) => {
        closed.push(sid);
      },
    };
    const app = await ready(slow);
    const opening = app.tabs.openConnection(MOCK_IDS.localZsh);
    await vi.waitFor(() => expect(app.tabs.tabs).toHaveLength(1));
    await app.tabs.close(app.tabs.tabs[0]?.id ?? "");
    release("late");
    await opening;
    expect(closed).toEqual(["late"]);
    expect(app.tabs.tabs).toHaveLength(0);
  });

  it("surfaces a real close failure but ignores unsupported/not_found", async () => {
    const base = workingIpc(createMockIpc({ vault: "unlocked" }));
    let code: "internal" | "not_found" = "internal";
    const ipc: Ipc = {
      ...base.ipc,
      sessionClose: () => Promise.reject(new IpcError(code, "boom")),
    };
    const app = await ready(ipc);
    const id = (await app.tabs.openConnection(MOCK_IDS.localZsh)) ?? "";
    await app.tabs.close(id);
    expect(app.notices.items).toHaveLength(1);

    code = "not_found";
    const id2 = (await app.tabs.openConnection(MOCK_IDS.localZsh)) ?? "";
    await app.tabs.close(id2);
    expect(app.notices.items).toHaveLength(1);
  });

  it("title follows renames and a missing connection fails the tab", async () => {
    const { ipc } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    await app.tabs.openConnection(MOCK_IDS.localZsh);
    const tab = app.tabs.tabs[0];
    expect(tab && app.tabs.title(tab)).toBe("Local shell");
    const node = app.doc.node(MOCK_IDS.localZsh);
    if (node) await app.doc.upsert({ ...node, name: "Renamed" });
    expect(tab && app.tabs.title(tab)).toBe("Renamed");

    await app.doc.remove(MOCK_IDS.localZsh);
    await app.tabs.retry(tab?.id ?? "");
    expect(app.tabs.tabs[0]?.error?.code).toBe("not_found");
  });

  it("navigation helpers change the active tab", async () => {
    const { ipc } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const a = (await app.tabs.openConnection(MOCK_IDS.localZsh)) ?? "";
    const b = (await app.tabs.openConnection(MOCK_IDS.devVm)) ?? "";
    app.tabs.next();
    expect(app.tabs.state.activeId).toBe(a);
    app.tabs.previous();
    expect(app.tabs.state.activeId).toBe(b);
    app.tabs.move(1, 0);
    expect(app.tabs.tabs.map((t) => t.id)).toEqual([b, a]);
    app.tabs.activate(a);
    expect(app.tabs.activeTab?.id).toBe(a);
  });

  it("a session component can fail its tab after open, which closes the session", async () => {
    const { ipc, closed } = workingIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const id = (await app.tabs.openConnection(MOCK_IDS.localZsh)) ?? "";
    await app.tabs.fail(id, { code: "io", message: "pty died" });
    expect(app.tabs.activeTab).toMatchObject({ status: "failed", sessionId: null });
    expect(closed).toEqual(["s1"]);
  });

  describe("credentials on demand (WP11)", () => {
    it("opens a connection without any credential at once: no vault, no unlock screen", async () => {
      // No vault was ever created and the seeded connections reference nothing.
      const { ipc, opened } = workingIpc(createMockIpc({ vault: "notCreated" }));
      const app = await ready(ipc);
      expect(app.vault.status).toBe("notCreated");
      expect(await app.tabs.openConnection(MOCK_IDS.localSshd)).not.toBeNull();
      expect(await app.tabs.openConnection(MOCK_IDS.rdpServer)).not.toBeNull();
      expect(opened).toEqual([`term:${MOCK_IDS.localSshd}`, `fb:${MOCK_IDS.rdpServer}`]);
      // Nothing asked for a vault, in any form.
      expect(app.vault.request).toBeNull();
      expect(app.ui.dialog).toBeNull();
    });

    it("treats a credential id that cannot exist (no vault) as no credential, without a nudge", async () => {
      const { ipc, opened } = workingIpc(createMockIpc({ vault: "notCreated" }));
      const app = await ready(ipc);
      const node = app.doc.node(MOCK_IDS.localSshd);
      if (!node || node.type !== "connection") throw new Error("seed changed");
      await app.doc.upsert({ ...node, credential: { type: "id", id: crypto.randomUUID() } });
      expect(await app.tabs.openConnection(MOCK_IDS.localSshd)).not.toBeNull();
      expect(app.vault.request).toBeNull();
      expect(opened).toHaveLength(1);
    });

    it("with a locked vault, 'enter manually' opens the session asking, and retry keeps that choice", async () => {
      const { ipc, opened } = workingIpc(createMockIpc({ vault: "locked" }));
      const app = await ready(ipc);
      const opening = app.tabs.openConnection(MOCK_IDS.localSshd);
      await vi.waitFor(() => expect(app.vault.request).not.toBeNull());
      expect(app.vault.manualAllowed).toBe(true);
      app.vault.finishRequestManual();
      const tabId = await opening;
      expect(tabId).not.toBeNull();
      expect(opened).toEqual([`term:${MOCK_IDS.localSshd}:ask`]);
      // The vault stays locked.
      expect(app.vault.unlocked).toBe(false);

      await app.tabs.retry(tabId ?? "");
      expect(opened.at(-1)).toBe(`term:${MOCK_IDS.localSshd}:ask`);
    });

    it("a tab that failed with vault_locked can switch to typing the credential", async () => {
      const base = createMockIpc({ vault: "unlocked" });
      const { ipc, opened } = workingIpc(base);
      let fail = true;
      const flaky: Ipc = {
        ...ipc,
        termOpen: (id, c, r, onEvent, options) => {
          if (fail) {
            fail = false;
            return Promise.reject(new IpcError("vault_locked", "the vault is locked"));
          }
          return ipc.termOpen(id, c, r, onEvent, options);
        },
      };
      const app = await ready(flaky);
      const tabId = await app.tabs.openConnection(MOCK_IDS.localSshd);
      expect(app.tabs.activeTab?.error?.code).toBe("vault_locked");
      await app.tabs.enterManuallyAndRetry(tabId ?? "");
      expect(app.tabs.activeTab?.status).toBe("open");
      expect(opened).toEqual([`term:${MOCK_IDS.localSshd}:ask`]);
    });

    it("an unlocked vault with the credential needs nothing: the session starts without asking", async () => {
      const { ipc, opened } = workingIpc(createMockIpc({ vault: "unlocked" }));
      const app = await ready(ipc);
      await app.tabs.openConnection(MOCK_IDS.localSshd);
      expect(opened).toEqual([`term:${MOCK_IDS.localSshd}`]);
      expect(app.vault.request).toBeNull();
    });
  });
});

describe("TabsStore liveness", () => {
  /** Opens succeed and keep each session's event callback, so tests can feed events. */
  function feedIpc(base: Ipc) {
    const feeds: Array<(e: unknown) => void> = [];
    let counter = 0;
    const ipc: Ipc = {
      ...base,
      termOpen: async (_id, _c, _r, onEvent) => {
        feeds.push(onEvent as (e: unknown) => void);
        return `s${++counter}`;
      },
      fbOpen: async (_id, _w, _h, onEvent) => {
        feeds.push(onEvent as (e: unknown) => void);
        return `s${++counter}`;
      },
      webOpen: async () => `s${++counter}`,
      sessionClose: async () => {},
    };
    return { ipc, feeds };
  }
  const data = { type: "data", bytes: new Uint8Array([1]) };
  const resize = { type: "resize", w: 1, h: 1 };

  it("counts only tabs that are connected", async () => {
    const { ipc, feeds } = feedIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const a = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string; // ssh
    const b = (await app.tabs.openConnection(MOCK_IDS.devVm)) as string; // vnc
    const w = (await app.tabs.openConnection(MOCK_IDS.intranet)) as string; // web
    expect(app.tabs.liveCount()).toBe(0);
    feeds[0]?.(data);
    expect(app.tabs.isLive(a)).toBe(true);
    expect(app.tabs.isLive(b)).toBe(false);
    feeds[1]?.(resize);
    expect(app.tabs.isLive(b)).toBe(true);
    expect(app.tabs.isLive(w)).toBe(false);
    expect(app.tabs.liveCount()).toBe(2);
    expect(app.tabs.liveCount([a, w])).toBe(1);
    feeds[0]?.({ type: "exit", code: 0 });
    expect(app.tabs.liveCount()).toBe(1);
  });

  it("resets a tab's state on close and on retry", async () => {
    const { ipc, feeds } = feedIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const a = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string;
    const b = (await app.tabs.openConnection(MOCK_IDS.devVm)) as string;
    feeds[0]?.(data);
    feeds[1]?.(resize);
    expect(app.tabs.liveCount()).toBe(2);
    await app.tabs.close(a);
    expect(app.tabs.isLive(a)).toBe(false);
    expect(app.tabs.liveCount()).toBe(1);
    await app.tabs.retry(b);
    expect(app.tabs.isLive(b)).toBe(false);
    // A late event of the old attempt does not revive it.
    feeds[1]?.(resize);
    expect(app.tabs.isLive(b)).toBe(false);
    feeds[2]?.(resize);
    expect(app.tabs.isLive(b)).toBe(true);
  });

  it("ignores events after fail() and resets on closeAll / closeForConnections", async () => {
    const { ipc, feeds } = feedIpc(createMockIpc({ vault: "unlocked" }));
    const app = await ready(ipc);
    const a = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string;
    feeds[0]?.(data);
    expect(app.tabs.isLive(a)).toBe(true);
    await app.tabs.fail(a, { code: "internal", message: "x" });
    expect(app.tabs.isLive(a)).toBe(false);
    feeds[0]?.(data);
    expect(app.tabs.isLive(a)).toBe(false);
    expect(app.tabs.liveCount()).toBe(0);

    const b = (await app.tabs.openConnection(MOCK_IDS.devVm)) as string;
    feeds[1]?.(resize);
    expect(app.tabs.isLive(b)).toBe(true);
    await app.tabs.closeForConnections(new Set([MOCK_IDS.devVm]));
    expect(app.tabs.isLive(b)).toBe(false);

    const c = (await app.tabs.openConnection(MOCK_IDS.localSshd)) as string;
    feeds[2]?.(data);
    expect(app.tabs.isLive(c)).toBe(true);
    await app.tabs.closeAll();
    expect(app.tabs.isLive(c)).toBe(false);
    expect(app.tabs.liveCount()).toBe(0);
  });
});
