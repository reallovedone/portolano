import { describe, expect, it } from "vitest";

import { MOCK_IDS, createMockIpc } from "./mock";

describe("mock import and export", () => {
  it("previews canned content per format and applies it under a folder, all or nothing", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    const preview = await ipc.importPreview("mobaxterm", "/x/Sessions.mxtsessions");
    expect(preview.nodes).toHaveLength(11);
    expect(preview.warnings.some((w) => w.message.includes("never imported"))).toBe(true);

    await expect(ipc.importApply(preview.previewId, "00000000-0000-4000-8000-0000000000ff")).rejects.toMatchObject({
      code: "not_found",
    });
    await expect(ipc.importApply(preview.previewId, MOCK_IDS.localZsh)).rejects.toMatchObject({
      code: "invalid_input",
    });
    const before = (await ipc.docGet()).nodes.length;
    expect(await ipc.importApply(preview.previewId, MOCK_IDS.production)).toBe(11);
    const after = (await ipc.docGet()).nodes;
    expect(after).toHaveLength(before + 11);
    expect(after.find((n) => n.name === "Jump box")?.parentId).toBe(MOCK_IDS.production);
    // A preview is single use.
    await expect(ipc.importApply(preview.previewId, null)).rejects.toMatchObject({ code: "not_found" });
  });

  it("answers with the error codes of the real backend for marked paths", async () => {
    const ipc = createMockIpc();
    await expect(ipc.importPreview("csv", "/x/missing.csv")).rejects.toMatchObject({ code: "io" });
    await expect(ipc.importPreview("mremoteng", "/x/encrypted.xml")).rejects.toMatchObject({
      code: "unsupported",
    });
    await expect(ipc.importPreview("json", "/x/broken.json")).rejects.toMatchObject({ code: "invalid_input" });
  });

  it("records exports and refuses .rdp for anything but one RDP connection", async () => {
    const ipc = createMockIpc();
    await ipc.exportDocument("json", "/x/all.json", null);
    await ipc.exportDocument("rdp", "/x/server.rdp", [MOCK_IDS.rdpServer]);
    expect(ipc.exports.map((e) => e.format)).toEqual(["json", "rdp"]);
    await expect(ipc.exportDocument("rdp", "/x/a.rdp", [MOCK_IDS.localSshd])).rejects.toMatchObject({
      code: "unsupported",
    });
    await expect(ipc.exportDocument("json", "/x/readonly.json", null)).rejects.toMatchObject({ code: "io" });
  });
});

describe("mock dangling references", () => {
  it("clears jump hosts when a connection is deleted", async () => {
    const ipc = createMockIpc();
    await ipc.docDeleteNode(MOCK_IDS.localSshd);
    const build = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_IDS.buildBox);
    expect(build).toMatchObject({ options: { type: "ssh", jumpHost: null } });
  });

  it("turns references to a deleted credential into none", async () => {
    const ipc = createMockIpc({ vault: "unlocked" });
    await ipc.vaultDelete(MOCK_IDS.credLab);
    const lab = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_IDS.lab);
    expect(lab).toMatchObject({ credential: { type: "none" } });
  });
});

describe("mock readPrivateKeyFile", () => {
  it("returns a fake key, and the real backend's error codes for marked paths", async () => {
    const ipc = createMockIpc();
    expect(await ipc.readPrivateKeyFile("/x/id_ed25519")).toContain("BEGIN OPENSSH PRIVATE KEY");
    await expect(ipc.readPrivateKeyFile("/x/missing_key")).rejects.toMatchObject({ code: "not_found" });
    await expect(ipc.readPrivateKeyFile("/x/unreadable")).rejects.toMatchObject({ code: "io" });
    await expect(ipc.readPrivateKeyFile("/x/big")).rejects.toMatchObject({ code: "invalid_input" });
    await expect(ipc.readPrivateKeyFile("/x/notakey")).rejects.toMatchObject({ code: "invalid_input" });
  });
});
