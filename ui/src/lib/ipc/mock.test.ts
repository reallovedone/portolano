import { describe, expect, it, vi } from "vitest";

import { IpcError } from "./errors";
import { createMockIpc, MOCK_MASTER_PASSWORD } from "./mock";
import type { Connection, Node } from "./types";

const newConnection = (id: string, parentId: string | null): Node => {
  const connection: Connection = {
    id,
    parentId,
    name: "c",
    host: "h",
    port: null,
    credential: { type: "inherit" },
    username: null,
    tags: [],
    notes: "",
    options: { type: "vnc", viewOnly: false },
  };
  return { type: "connection", ...connection };
};

async function codeOf(p: Promise<unknown>): Promise<string> {
  try {
    await p;
  } catch (e) {
    if (e instanceof IpcError) return e.code;
    throw e;
  }
  return "no error";
}

describe("mock document", () => {
  it("returns the seed and upserts, moves and deletes nodes", async () => {
    const ipc = createMockIpc();
    const seed = await ipc.docGet();
    expect(seed.schemaVersion).toBe(1);
    expect(seed.nodes.length).toBeGreaterThan(0);

    const id = "00000000-0000-4000-8000-0000000000aa";
    await ipc.docUpsertNode(newConnection(id, null));
    expect((await ipc.docGet()).nodes.some((n) => n.id === id)).toBe(true);

    const folderId = seed.nodes.find((n) => n.type === "folder")?.id ?? "";
    await ipc.docMoveNode(id, folderId);
    expect((await ipc.docGet()).nodes.find((n) => n.id === id)?.parentId).toBe(folderId);

    await ipc.docDeleteNode(folderId);
    const after = await ipc.docGet();
    expect(after.nodes.some((n) => n.id === id)).toBe(false);
  });

  it("rejects unknown nodes and cycles", async () => {
    const ipc = createMockIpc({ seed: false });
    expect(await codeOf(ipc.docDeleteNode("missing"))).toBe("not_found");
    const folder: Node = {
      type: "folder",
      id: "00000000-0000-4000-8000-0000000000f1",
      parentId: null,
      name: "f",
      credential: { type: "inherit" },
    };
    await ipc.docUpsertNode(folder);
    expect(await codeOf(ipc.docMoveNode(folder.id, folder.id))).toBe("invalid_input");
  });

  it("does not leak internal state through returned values", async () => {
    const ipc = createMockIpc();
    const doc = await ipc.docGet();
    doc.nodes.length = 0;
    expect((await ipc.docGet()).nodes.length).toBeGreaterThan(0);
  });
});

describe("mock vault", () => {
  it("follows notCreated -> unlocked -> locked -> unlocked", async () => {
    const ipc = createMockIpc();
    expect(await ipc.vaultStatus()).toBe("notCreated");
    expect(await codeOf(ipc.vaultList())).toBe("vault_locked");

    await ipc.vaultCreate("pw");
    expect(await ipc.vaultStatus()).toBe("unlocked");

    const summary = await ipc.vaultUpsert({
      id: null,
      name: "n",
      username: "u",
      domain: null,
      secret: { type: "password", password: "s3cret" },
    });
    expect(JSON.stringify(summary)).not.toContain("s3cret");
    expect(summary.kind).toBe("password");
    expect(await ipc.vaultList()).toEqual([summary]);

    await ipc.vaultLock();
    expect(await ipc.vaultStatus()).toBe("locked");
    expect(await codeOf(ipc.vaultRevealSecret(summary.id))).toBe("vault_locked");
    expect(await codeOf(ipc.vaultUnlock("wrong"))).toBe("auth_failed");

    await ipc.vaultUnlock("pw");
    expect(await ipc.vaultRevealSecret(summary.id)).toEqual({
      type: "password",
      password: "s3cret",
    });
  });

  it("changes the master password", async () => {
    const ipc = createMockIpc();
    await ipc.vaultCreate("a");
    expect(await codeOf(ipc.vaultChangePassword("x", "b"))).toBe("auth_failed");
    await ipc.vaultChangePassword("a", "b");
    await ipc.vaultLock();
    await ipc.vaultUnlock("b");
    expect(await ipc.vaultStatus()).toBe("unlocked");
  });
});

describe("mock auto-lock time", () => {
  it("defaults to 15 minutes, keeps a valid choice and refuses any other", async () => {
    const ipc = createMockIpc();
    expect(await ipc.vaultGetAutoLock()).toBe(15);
    await ipc.vaultSetAutoLock(0);
    expect(await ipc.vaultGetAutoLock()).toBe(0);
    expect(await codeOf(ipc.vaultSetAutoLock(7))).toBe("invalid_input");
    expect(await ipc.vaultGetAutoLock()).toBe(0);
    expect(await createMockIpc({ autoLockMinutes: 60 }).vaultGetAutoLock()).toBe(60);
  });
});

describe("mock quick unlock", () => {
  async function ready() {
    const ipc = createMockIpc({ vault: "locked" });
    await ipc.vaultSetQuickUnlock(true);
    await ipc.vaultUnlock(MOCK_MASTER_PASSWORD);
    await ipc.vaultLock();
    return ipc;
  }

  it("is off, available and not ready at the start", async () => {
    const ipc = createMockIpc({ vault: "locked" });
    expect(await ipc.vaultQuickUnlockInfo()).toEqual({ available: true, enabled: false, ready: false });
    expect(await createMockIpc({ quickUnlockAvailable: false }).vaultQuickUnlockInfo()).toEqual({
      available: false,
      enabled: false,
      ready: false,
    });
  });

  it("becomes ready after a password unlock with the option on, and survives Lock", async () => {
    const ipc = createMockIpc({ vault: "locked" });
    await ipc.vaultSetQuickUnlock(true);
    expect((await ipc.vaultQuickUnlockInfo()).ready).toBe(false);
    await ipc.vaultUnlock(MOCK_MASTER_PASSWORD);
    expect(await ipc.vaultQuickUnlockInfo()).toEqual({ available: true, enabled: true, ready: true });
    await ipc.vaultLock();
    expect((await ipc.vaultQuickUnlockInfo()).ready).toBe(true);
    await ipc.vaultQuickUnlock();
    expect(await ipc.vaultStatus()).toBe("unlocked");
    expect(ipc.quickUnlock.prompts).toBe(1);
  });

  it("is dropped by turning the option off and by changing the password", async () => {
    const off = await ready();
    await off.vaultSetQuickUnlock(false);
    expect(await codeOf(off.vaultQuickUnlock())).toBe("quick_unlock_not_ready");
    await off.vaultSetQuickUnlock(true);
    expect((await off.vaultQuickUnlockInfo()).ready).toBe(false);

    const changed = await ready();
    await changed.vaultUnlock(MOCK_MASTER_PASSWORD);
    await changed.vaultChangePassword(MOCK_MASTER_PASSWORD, "other");
    expect((await changed.vaultQuickUnlockInfo()).ready).toBe(false);
    await changed.vaultLock();
    expect(await codeOf(changed.vaultQuickUnlock())).toBe("quick_unlock_not_ready");
    expect(await changed.vaultStatus()).toBe("locked");
  });

  it("captures nothing with the option off", async () => {
    const ipc = createMockIpc({ vault: "locked" });
    await ipc.vaultUnlock(MOCK_MASTER_PASSWORD);
    await ipc.vaultLock();
    expect((await ipc.vaultQuickUnlockInfo()).ready).toBe(false);
    expect(await codeOf(ipc.vaultQuickUnlock())).toBe("quick_unlock_not_ready");
    expect(ipc.quickUnlock.prompts).toBe(0);
  });

  it("reports a cancelled or failed prompt and a failed unlock like the backend", async () => {
    const ipc = await ready();
    ipc.quickUnlock.outcome = "cancelled";
    expect(await codeOf(ipc.vaultQuickUnlock())).toBe("quick_unlock_cancelled");
    ipc.quickUnlock.outcome = "failed";
    expect(await codeOf(ipc.vaultQuickUnlock())).toBe("quick_unlock_failed");
    expect((await ipc.vaultQuickUnlockInfo()).ready).toBe(true);
    ipc.quickUnlock.outcome = "unlock-failed";
    expect(await codeOf(ipc.vaultQuickUnlock())).toBe("quick_unlock_failed");
    expect((await ipc.vaultQuickUnlockInfo()).ready).toBe(false);
    expect(await ipc.vaultStatus()).toBe("locked");
  });

  it("refuses to turn on without a biometric method", async () => {
    const ipc = createMockIpc({ quickUnlockAvailable: false });
    expect(await codeOf(ipc.vaultSetQuickUnlock(true))).toBe("unsupported");
    await ipc.vaultSetQuickUnlock(false);
  });
});

describe("mock sessions", () => {
  it("rejects with unsupported", async () => {
    const ipc = createMockIpc();
    expect(await codeOf(ipc.sessionClose("x"))).toBe("unsupported");
    expect(await ipc.knownHostsList()).toEqual([]);
  });
});

describe("mock quit", () => {
  it("records the replies and plays quit-requested to its subscribers", async () => {
    const ipc = createMockIpc();
    const seen = vi.fn();
    const off = ipc.onQuitRequested(seen);
    ipc.simulate.quitRequested();
    expect(seen).toHaveBeenCalledTimes(1);
    await ipc.quitReply("asking");
    await ipc.quitReply("quit");
    expect(ipc.quitReplies).toEqual(["asking", "quit"]);
    off();
    ipc.simulate.quitRequested();
    expect(seen).toHaveBeenCalledTimes(1);
  });
});
