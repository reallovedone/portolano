import { describe, expect, it, vi } from "vitest";

import { IpcError } from "./errors";
import { createMockIpc, MOCK_MASTER_PASSWORD } from "./mock";
import { MOCK_SHARE_IDS } from "./mock-shares";
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

describe("mock node appearance", () => {
  const id = "00000000-0000-4000-8000-0000000000b1";

  it("stores a known icon and colour and rejects unknown ones", async () => {
    const ipc = createMockIpc();
    const base = newConnection(id, null);
    const stored = await ipc.docUpsertNode({ ...base, icon: "server", color: "teal" });
    expect(stored).toMatchObject({ icon: "server", color: "teal" });
    expect((await ipc.docGet()).nodes.find((n) => n.id === id)).toMatchObject({ icon: "server", color: "teal" });

    expect(await codeOf(ipc.docUpsertNode({ ...base, icon: "not-an-icon" }))).toBe("invalid_input");
    expect(await codeOf(ipc.docUpsertNode({ ...base, icon: "" }))).toBe("invalid_input");
    expect(await codeOf(ipc.docUpsertNode({ ...base, color: "magenta" }))).toBe("invalid_input");
    expect(await codeOf(ipc.docUpsertNode({ ...base, color: "" }))).toBe("invalid_input");
    // The rejected saves changed nothing.
    expect((await ipc.docGet()).nodes.find((n) => n.id === id)).toMatchObject({ icon: "server", color: "teal" });
  });

  it("validates folders the same way", async () => {
    const ipc = createMockIpc();
    const folder: Node = { type: "folder", id, parentId: null, name: "f", credential: { type: "inherit" } };
    expect(await codeOf(ipc.docUpsertNode({ ...folder, icon: "folder", color: "gray" }))).toBe("no error");
    expect(await codeOf(ipc.docUpsertNode({ ...folder, color: "mauve" }))).toBe("invalid_input");
  });
});

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

  it("reorders nodes inside and across folders and sorts them back by name", async () => {
    const ipc = createMockIpc({ seed: false });
    const mk = (n: number, type: "folder" | "connection", parentId: string | null, name: string): Node => {
      const id = `00000000-0000-4000-8000-0000000001${String(n).padStart(2, "0")}`;
      return type === "folder"
        ? { type, id, parentId, name, credential: { type: "inherit" } }
        : { ...newConnection(id, parentId), name };
    };
    const folderA = mk(1, "folder", null, "A");
    const folderB = mk(2, "folder", null, "B");
    const [a, b, c] = [3, 4, 5].map((n, i) => mk(n, "connection", folderA.id, ["a", "b", "c"][i] ?? ""));
    for (const node of [folderA, folderB, a, b, c]) await ipc.docUpsertNode(node);
    const index = async (id: string) => (await ipc.docGet()).nodes.find((n) => n.id === id)?.sortIndex;

    // c before a: c a b, contiguous indices; the folders keep no index.
    await ipc.nodeReorder(c.id, folderA.id, a.id);
    expect([await index(c.id), await index(a.id), await index(b.id)]).toEqual([0, 1, 2]);
    expect(await index(folderA.id)).toBeUndefined();
    // No "before": to the end of the group.
    await ipc.nodeReorder(c.id, folderA.id, null);
    expect([await index(a.id), await index(b.id), await index(c.id)]).toEqual([0, 1, 2]);
    // Across parents.
    await ipc.nodeReorder(a.id, folderB.id, null);
    const moved = (await ipc.docGet()).nodes.find((n) => n.id === a.id);
    expect([moved?.parentId, moved?.sortIndex]).toEqual([folderB.id, 0]);
    // A hint decides the first numbering of an unindexed group; an indexed one ignores it.
    const folderC = mk(9, "folder", null, "C");
    await ipc.docUpsertNode(folderC);
    const [x, y, z] = [6, 7, 8].map((n, i) =>
      mk(n, "connection", folderC.id, ["x", "y", "z"][i] ?? ""),
    );
    for (const node of [x, y, z]) await ipc.docUpsertNode(node);
    await ipc.nodeReorder(x.id, folderC.id, null, [z.id, y.id, x.id, "unknown"]);
    expect([await index(z.id), await index(y.id), await index(x.id)]).toEqual([0, 1, 2]);
    await ipc.nodeReorder(z.id, folderC.id, null, [x.id, y.id, z.id]);
    expect([await index(y.id), await index(x.id), await index(z.id)]).toEqual([0, 1, 2]);
    // Errors: not a sibling, not the same kind, unknown node, cycle.
    expect(await codeOf(ipc.nodeReorder(b.id, folderA.id, a.id))).toBe("invalid_input");
    expect(await codeOf(ipc.nodeReorder(b.id, folderA.id, folderB.id))).toBe("invalid_input");
    expect(await codeOf(ipc.nodeReorder("missing", null, null))).toBe("not_found");
    expect(await codeOf(ipc.nodeReorder(folderA.id, folderA.id, null))).toBe("invalid_input");
    // Sorting one parent only clears that parent's indices.
    await ipc.nodeSortByName(folderA.id);
    expect(await index(b.id)).toBeUndefined();
    expect(await index(a.id)).toBe(0);
    expect(await codeOf(ipc.nodeSortByName(a.id))).toBe("invalid_input");
  });

  it("keeps the stored order when a node is edited and drops it when the parent changes", async () => {
    const ipc = createMockIpc({ seed: false });
    const folder: Node = {
      type: "folder",
      id: "00000000-0000-4000-8000-0000000002f1",
      parentId: null,
      name: "f",
      credential: { type: "inherit" },
    };
    const a = { ...newConnection("00000000-0000-4000-8000-0000000002a1", null), name: "a" };
    const b = { ...newConnection("00000000-0000-4000-8000-0000000002a2", null), name: "b" };
    for (const node of [folder, a, b]) await ipc.docUpsertNode(node);
    await ipc.nodeReorder(b.id, null, a.id);
    const find = async (id: string) => (await ipc.docGet()).nodes.find((n) => n.id === id);
    expect((await find(b.id))?.sortIndex).toBe(0);
    // An edit that carries no index, or a wrong one, keeps the stored index.
    await ipc.docUpsertNode({ ...b, name: "b2" });
    expect((await find(b.id))?.sortIndex).toBe(0);
    await ipc.docUpsertNode({ ...b, name: "b3", sortIndex: 9 });
    expect((await find(b.id))?.sortIndex).toBe(0);
    // A parent change drops it.
    await ipc.docUpsertNode({ ...b, parentId: folder.id, sortIndex: 5 });
    expect((await find(b.id))?.sortIndex).toBeUndefined();
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

describe("mock appearance", () => {
  const dark = { theme: "dark", scheme: "nord", accent: "teal", density: "compact", terminalFontSize: 16 } as const;

  it("starts from the defaults and keeps what is saved", async () => {
    const ipc = createMockIpc();
    expect(await ipc.appearanceGet()).toEqual({
      theme: "system",
      scheme: "portolano",
      accent: "blue",
      density: "comfortable",
      terminalFontSize: 13,
    });
    expect(await ipc.appearanceSet(dark)).toEqual(dark);
    expect(await ipc.appearanceGet()).toEqual(dark);
  });

  it("refuses values outside the lists and changes nothing", async () => {
    const ipc = createMockIpc({ appearance: dark });
    await expect(ipc.appearanceSet({ ...dark, accent: "pink" as never })).rejects.toThrow();
    await expect(ipc.appearanceSet({ ...dark, scheme: "dracula" as never })).rejects.toThrow();
    await expect(ipc.appearanceSet({ ...dark, terminalFontSize: 9 })).rejects.toThrow();
    await expect(ipc.appearanceSet({ ...dark, terminalFontSize: 25 })).rejects.toThrow();
    await expect(ipc.appearanceSet({ ...dark, terminalFontSize: 12.5 })).rejects.toThrow();
    expect(await ipc.appearanceGet()).toEqual(dark);
  });

  it("tells subscribers when the Settings menu item is chosen", () => {
    const ipc = createMockIpc();
    const handler = vi.fn();
    const off = ipc.onSettingsMenu(handler);
    ipc.simulate.settingsMenu();
    expect(handler).toHaveBeenCalledTimes(1);
    off();
    ipc.simulate.settingsMenu();
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("tells subscribers when the Release Notes menu item is chosen", () => {
    const ipc = createMockIpc();
    const handler = vi.fn();
    const off = ipc.onReleaseNotesMenu(handler);
    ipc.simulate.releaseNotesMenu();
    expect(handler).toHaveBeenCalledTimes(1);
    off();
    ipc.simulate.releaseNotesMenu();
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("returns the release notes of every version, or the failure it was told to", async () => {
    const ipc = createMockIpc();
    const list = await ipc.releaseNotes();
    expect(list.sections.map((s) => s.version)).toEqual(["0.2.0", "0.1.0"]);
    expect(list.truncated).toBe(false);
    expect(ipc.updates.releaseNotesCalls).toBe(1);
    const custom = { sections: [{ version: "1.0.0", date: null, notes: "- x" }], truncated: true };
    expect(await createMockIpc({ updates: { releaseNotes: custom } }).releaseNotes()).toEqual(custom);
    await expect(createMockIpc({ updates: { releaseNotesError: "boom" } }).releaseNotes()).rejects.toThrow("boom");
  });
});

describe("mock update settings", () => {
  it("keeps pre-releases off by default and remembers the choice", async () => {
    const ipc = createMockIpc();
    expect((await ipc.updateSettings()).includePrereleases).toBe(false);
    await ipc.updateSetIncludePrereleases(true);
    expect(await ipc.updateSettings()).toMatchObject({ includePrereleases: true, autoCheck: true });
    expect((await createMockIpc({ updates: { includePrereleases: true } }).updateSettings()).includePrereleases).toBe(true);
  });
});

describe("mock shared folders", () => {
  it("mock_share_set_credential_updates_view", async () => {
    const ipc = createMockIpc({ shares: true });
    const view = await ipc.docGet();
    expect(view.shares).toHaveLength(1);
    expect(view.shares[0]).toMatchObject({ id: MOCK_SHARE_IDS.link, role: "subscriber", pendingTargets: 1 });
    expect(view.remoteNodes[MOCK_SHARE_IDS.root]).toEqual({ linkId: MOCK_SHARE_IDS.link, isRoot: true });
    expect(view.remoteNodes[MOCK_SHARE_IDS.gateway]).toEqual({ linkId: MOCK_SHARE_IDS.link, isRoot: false });
    expect(view.nodes.find((n) => n.id === MOCK_SHARE_IDS.gateway)?.credential).toEqual({ type: "inherit" });

    await ipc.shareSetCredential(MOCK_SHARE_IDS.gateway, { type: "none" });
    expect((await ipc.docGet()).nodes.find((n) => n.id === MOCK_SHARE_IDS.gateway)?.credential).toEqual({
      type: "none",
    });
    await ipc.shareSetCredential(MOCK_SHARE_IDS.gateway, { type: "inherit" });
    expect((await ipc.docGet()).nodes.find((n) => n.id === MOCK_SHARE_IDS.gateway)?.credential).toEqual({
      type: "inherit",
    });
    // A local node is not a shared one.
    expect(await codeOf(ipc.shareSetCredential("00000000-0000-4000-8000-000000000001", { type: "none" }))).toBe(
      "invalid_input",
    );
  });

  it("has no shares unless asked and keeps remote nodes read-only", async () => {
    expect((await createMockIpc().docGet()).shares).toEqual([]);
    const ipc = createMockIpc({ shares: true });
    const gateway = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_SHARE_IDS.gateway) as Node;
    expect(await codeOf(ipc.docUpsertNode({ ...gateway, name: "x" }))).toBe("read_only");
    expect(await codeOf(ipc.docDeleteNode(MOCK_SHARE_IDS.gateway))).toBe("read_only");
    expect(await codeOf(ipc.docMoveNode(MOCK_SHARE_IDS.gateway, null))).toBe("read_only");
    expect(await codeOf(ipc.docMoveNode("00000000-0000-4000-8000-000000000001", MOCK_SHARE_IDS.servers))).toBe(
      "read_only",
    );
    // The mount root moves into a local folder.
    await ipc.docMoveNode(MOCK_SHARE_IDS.root, "00000000-0000-4000-8000-000000000009");
    const moved = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_SHARE_IDS.root);
    expect(moved?.parentId).toBe("00000000-0000-4000-8000-000000000009");
  });

  it("refuses to delete a folder that holds a share, then allows it once unsubscribed", async () => {
    const ipc = createMockIpc({ shares: true });
    const production = "00000000-0000-4000-8000-000000000009";
    await ipc.docMoveNode(MOCK_SHARE_IDS.root, production);
    const error = await ipc.docDeleteNode(production).catch((e: unknown) => e);
    expect(error).toMatchObject({ code: "conflict", message: "share.deleteContainsShares" });
    await ipc.shareUnsubscribe(MOCK_SHARE_IDS.link);
    await ipc.docDeleteNode(production);
    const view = await ipc.docGet();
    expect(view.shares).toEqual([]);
    expect(view.nodes.some((n) => n.id === MOCK_SHARE_IDS.root)).toBe(false);
  });

  it("confirms the targets only for the revision the user saw", async () => {
    const ipc = createMockIpc({ shares: true });
    expect(await codeOf(ipc.shareConfirmTargets(MOCK_SHARE_IDS.link, 1))).toBe("conflict");
    expect((await ipc.docGet()).shares[0]?.pendingTargets).toBe(1);
    await ipc.shareConfirmTargets(MOCK_SHARE_IDS.link, 3);
    expect((await ipc.docGet()).shares[0]?.pendingTargets).toBe(0);
  });

  it("emits shares-changed to subscribers", () => {
    const ipc = createMockIpc();
    const handler = vi.fn();
    const off = ipc.onSharesChanged(handler);
    ipc.simulate.sharesChanged();
    off();
    ipc.simulate.sharesChanged();
    expect(handler).toHaveBeenCalledTimes(1);
  });
});
