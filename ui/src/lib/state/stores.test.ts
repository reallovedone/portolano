import { describe, expect, it, vi } from "vitest";

import { IpcError, type Ipc } from "../ipc";
import { MOCK_IDS, MOCK_MASTER_PASSWORD, createMockIpc } from "../ipc/mock";
import { createApp } from "./app";
import { SessionBridge } from "../tabs/session";
import { isNumber, isStringArray, loadJson, saveJson, type KeyValueStore } from "./storage";

function memoryStore(initial: Record<string, string> = {}): KeyValueStore & { data: Record<string, string> } {
  const data = { ...initial };
  return {
    data,
    getItem: (k) => data[k] ?? null,
    setItem: (k, v) => {
      data[k] = v;
    },
  };
}

describe("vault flow against the mock", () => {
  it("creates, locks, rejects a wrong password and unlocks", async () => {
    const app = createApp(createMockIpc());
    await app.vault.refresh();
    expect(app.vault.status).toBe("notCreated");

    expect((await app.vault.create("correct horse")).ok).toBe(true);
    expect(app.vault.status).toBe("unlocked");

    expect((await app.vault.lock()).ok).toBe(true);
    expect(app.vault.status).toBe("locked");
    expect(app.vault.credentials).toEqual([]);

    const wrong = await app.vault.unlock("nope");
    expect(wrong.ok).toBe(false);
    if (!wrong.ok) expect(wrong.error.code).toBe("auth_failed");
    expect(app.vault.status).toBe("locked");

    expect((await app.vault.unlock("correct horse")).ok).toBe(true);
    expect(app.vault.status).toBe("unlocked");
  });

  it("reports vault_locked while locked and notices a lock behind our back", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    await app.vault.refresh();
    expect(app.vault.credentials.length).toBe(3);

    await ipc.vaultLock(); // e.g. an automatic lock in the backend
    const result = await app.vault.upsert({
      id: null,
      name: "x",
      username: "u",
      domain: null,
      secret: { type: "password", password: "p" },
    });
    expect(result.ok).toBe(false);
    expect(app.vault.status).toBe("locked");
    expect(app.vault.credentials).toEqual([]);
  });

  it("keeps the secret when only metadata changes and never stores it", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const app = createApp(ipc);
    await app.vault.refresh();
    const saved = await app.vault.upsertKeepingSecret({
      id: MOCK_IDS.credLab,
      name: "Renamed",
      username: "admin",
      domain: null,
    });
    expect(saved.ok).toBe(true);
    expect(app.vault.name(MOCK_IDS.credLab)).toBe("Renamed");
    expect(await ipc.vaultRevealSecret(MOCK_IDS.credLab)).toEqual({
      type: "password",
      password: "mock-lab-password",
    });
    expect(JSON.stringify(app.vault.credentials)).not.toContain("mock-lab-password");
  });

  it("ensureUnlocked resolves immediately when unlocked, and waits for the unlock screen otherwise", async () => {
    const app = createApp(createMockIpc({ vault: "locked" }));
    await app.vault.refresh();

    const pending = app.vault.ensureUnlocked("Build box");
    expect(app.vault.request).toEqual({ reason: "Build box" });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    app.vault.finishRequest(true);
    expect(await pending).toBe(true);
    expect(app.vault.request).toBeNull();

    expect(await app.vault.ensureUnlocked()).toBe(true);
    expect(app.vault.request).toBeNull();
  });

  it("cancelling the unlock screen resolves false; a newer request supersedes an older one", async () => {
    const app = createApp(createMockIpc({ vault: "locked" }));
    await app.vault.refresh();
    const first = app.vault.ensureUnlocked("a");
    const second = app.vault.ensureUnlocked("b");
    expect(await first).toBe(false);
    app.vault.finishRequest(false);
    expect(await second).toBe(false);
  });

  it("ensureUnlockedOrManual offers 'enter manually' and reports how the request ended", async () => {
    const app = createApp(createMockIpc({ vault: "locked" }));
    await app.vault.refresh();

    const manual = app.vault.ensureUnlockedOrManual("Build box");
    expect(app.vault.request).toEqual({ reason: "Build box" });
    expect(app.vault.manualAllowed).toBe(true);
    app.vault.finishRequestManual();
    expect(await manual).toBe("manual");
    expect(app.vault.request).toBeNull();
    expect(app.vault.manualAllowed).toBe(false);
    // The vault was not touched.
    expect(app.vault.unlocked).toBe(false);

    const cancelled = app.vault.ensureUnlockedOrManual("x");
    app.vault.finishRequest(false);
    expect(await cancelled).toBe("cancelled");

    const unlocked = app.vault.ensureUnlockedOrManual("y");
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    app.vault.finishRequest(true);
    expect(await unlocked).toBe("unlocked");

    // The plain request never offers the alternative.
    await app.vault.lock();
    const plain = app.vault.ensureUnlocked("z");
    expect(app.vault.manualAllowed).toBe(false);
    app.vault.finishRequest(false);
    expect(await plain).toBe(false);
  });

  it("changes the master password through the mock", async () => {
    const app = createApp(createMockIpc({ vault: "unlocked" }));
    const bad = await app.vault.changePassword("wrong", "new-secret-1");
    expect(bad.ok).toBe(false);
    expect((await app.vault.changePassword(MOCK_MASTER_PASSWORD, "new-secret-1")).ok).toBe(true);
    await app.vault.lock();
    expect((await app.vault.unlock("new-secret-1")).ok).toBe(true);
  });
});

describe("document store", () => {
  it("loads, upserts, moves and cascades deletes, remembering expansion", async () => {
    const store = memoryStore();
    vi.stubGlobal("localStorage", store);
    try {
      const app = createApp(createMockIpc());
      expect(await app.doc.load()).toBe(true);
      expect(app.doc.status).toBe("ready");
      const before = app.doc.nodes.length;

      const moved = await app.doc.move(MOCK_IDS.localZsh, MOCK_IDS.production);
      expect(moved.ok).toBe(true);
      expect(app.doc.node(MOCK_IDS.localZsh)?.parentId).toBe(MOCK_IDS.production);
      expect(app.doc.isExpanded(MOCK_IDS.production)).toBe(true);
      expect(JSON.parse(store.data["portolano.tree.expanded"] ?? "[]")).toContain(MOCK_IDS.production);

      const invalid = await app.doc.move(MOCK_IDS.lab, MOCK_IDS.buildBox);
      expect(invalid.ok).toBe(false);

      const removed = await app.doc.remove(MOCK_IDS.lab);
      expect(removed.ok).toBe(true);
      expect(app.doc.node(MOCK_IDS.buildBox)).toBeUndefined();
      expect(app.doc.nodes.length).toBeLessThan(before);

      app.doc.collapseAll();
      expect(app.doc.expanded.size).toBe(0);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("expandAll opens every folder and persists it", async () => {
    const store = memoryStore();
    vi.stubGlobal("localStorage", store);
    try {
      const app = createApp(createMockIpc());
      expect(await app.doc.load()).toBe(true);
      const folderIds = app.doc.nodes.filter((n) => n.type === "folder").map((n) => n.id);
      expect(folderIds.length).toBeGreaterThan(0);

      app.doc.expandAll();
      for (const id of folderIds) expect(app.doc.isExpanded(id)).toBe(true);
      expect(app.doc.expanded.size).toBe(folderIds.length);

      const saved = JSON.parse(store.data["portolano.tree.expanded"] ?? "[]") as string[];
      expect([...saved].sort()).toEqual([...folderIds].sort());
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("exposes a load failure with its code", async () => {
    const ipc: Ipc = {
      ...createMockIpc(),
      docGet: () => Promise.reject(new IpcError("io", "disk on fire")),
    };
    const app = createApp(ipc);
    expect(await app.doc.load()).toBe(false);
    expect(app.doc.status).toBe("error");
    expect(app.doc.loadError?.code).toBe("io");
  });

  it("returns upsert errors instead of throwing", async () => {
    const app = createApp(createMockIpc({ seed: false }));
    await app.doc.load();
    const result = await app.doc.upsert({
      type: "folder",
      id: "f",
      parentId: "missing",
      name: "x",
      credential: { type: "inherit" },
    });
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe("not_found");
  });
});

describe("overlay and notices", () => {
  it("tracks open overlays with idempotent release", () => {
    const { overlay } = createApp(createMockIpc());
    expect(overlay.open).toBe(false);
    const a = overlay.acquire();
    const b = overlay.acquire();
    expect(overlay.open).toBe(true);
    a();
    a();
    expect(overlay.open).toBe(true);
    b();
    expect(overlay.open).toBe(false);
  });

  it("formats IPC errors and auto-dismisses info notices", () => {
    vi.useFakeTimers();
    try {
      const { notices } = createApp(createMockIpc());
      notices.error("Failed", new IpcError("not_found", "gone"));
      expect(notices.items[0]).toMatchObject({ kind: "error", title: "Failed", detail: "gone (not_found)" });
      notices.push("info", "hello");
      expect(notices.items.length).toBe(2);
      vi.advanceTimersByTime(5100);
      expect(notices.items.map((n) => n.kind)).toEqual(["error"]);
      const first = notices.items[0];
      if (first) notices.dismiss(first.id);
      expect(notices.items).toEqual([]);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("storage helpers", () => {
  it("falls back on missing, corrupt or wrongly typed values", () => {
    expect(loadJson("k", [], isStringArray, memoryStore())).toEqual([]);
    expect(loadJson("k", 7, isNumber, memoryStore({ k: "{oops" }))).toBe(7);
    expect(loadJson("k", [], isStringArray, memoryStore({ k: "[1,2]" }))).toEqual([]);
    expect(loadJson("k", [], isStringArray, memoryStore({ k: '["a"]' }))).toEqual(["a"]);
    expect(loadJson("k", 1, isNumber, null)).toBe(1);
  });

  it("reports failed writes instead of throwing", () => {
    const failing: KeyValueStore = {
      getItem: () => null,
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(saveJson("k", 1, failing)).toBe(false);
    expect(saveJson("k", 1, null)).toBe(false);
    expect(saveJson("k", 1, memoryStore())).toBe(true);
  });
});

describe("SessionBridge", () => {
  it("buffers events until the first subscriber, then forwards live", () => {
    const bridge = new SessionBridge();
    bridge.emit({ type: "exit", code: 0 });
    bridge.emit({ type: "exit", code: 1 });
    const seen: unknown[] = [];
    const unsubscribe = bridge.subscribe((e) => seen.push(e));
    expect(seen).toHaveLength(2);
    bridge.emit({ type: "exit", code: 2 });
    expect(seen).toHaveLength(3);
    unsubscribe();
    bridge.emit({ type: "exit", code: 3 });
    expect(seen).toHaveLength(3);
    const again: unknown[] = [];
    bridge.subscribe((e) => again.push(e));
    expect(again).toHaveLength(1);
  });
});
