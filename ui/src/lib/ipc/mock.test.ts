import { describe, expect, it } from "vitest";

import { IpcError } from "./errors";
import { createMockIpc } from "./mock";
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

describe("mock sessions", () => {
  it("rejects with unsupported", async () => {
    const ipc = createMockIpc();
    expect(await codeOf(ipc.sessionClose("x"))).toBe("unsupported");
    expect(await ipc.knownHostsList()).toEqual([]);
  });
});
