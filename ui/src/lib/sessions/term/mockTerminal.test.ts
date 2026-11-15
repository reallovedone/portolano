import { describe, expect, it } from "vitest";

import { IpcError, type TermEvent } from "../../ipc";
import { MOCK_IDS, createMockIpc } from "../../ipc/mock";

const text = (events: TermEvent[]): string =>
  events
    .filter((e): e is Extract<TermEvent, { type: "data" }> => e.type === "data")
    .map((e) => new TextDecoder().decode(e.bytes))
    .join("");

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

describe("fake terminal of the mock IPC", () => {
  it("is off by default", async () => {
    const ipc = createMockIpc();
    await expect(ipc.termOpen(MOCK_IDS.localZsh, 80, 24, () => undefined)).rejects.toMatchObject({
      code: "unsupported",
    });
  });

  it("echoes typed lines, handles backspace and ends on `exit`", async () => {
    const ipc = createMockIpc({ terminal: true });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localZsh, 80, 24, (e) => events.push(e));
    await tick();
    expect(text(events)).toContain("Portolano mock terminal");
    const enc = new TextEncoder();
    await ipc.termWrite(id, enc.encode("helx\x7flo\r"));
    expect(text(events)).toContain("mock: hello");
    await ipc.termWrite(id, enc.encode("exit\r"));
    expect(events.at(-1)).toEqual({ type: "exit", code: 0 });
    await ipc.termAck(id, 10);
    await ipc.termResize(id, 100, 30);
  });

  it("asks for the host key of an unknown SSH host and remembers an accepted one", async () => {
    // Unlocked vault: the host gets its credential from it, so no credential prompt follows.
    const ipc = createMockIpc({ terminal: true, vault: "unlocked" });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e));
    await tick();
    expect(events[0]).toMatchObject({ type: "hostKeyPrompt", algorithm: "ssh-ed25519" });
    await ipc.sshHostKeyDecision(id, true);
    expect(text(events)).toContain("127.0.0.1");
    expect(await ipc.knownHostsList()).toEqual(
      expect.arrayContaining([expect.objectContaining({ host: "127.0.0.1", port: 12222 })]),
    );

    // Second session: the key is known, no prompt.
    const again: TermEvent[] = [];
    await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => again.push(e));
    await tick();
    expect(again.some((e) => e.type === "hostKeyPrompt")).toBe(false);
    expect(text(again)).toContain("Portolano mock terminal");
  });

  it("a rejected host key ends with auth_failed", async () => {
    const ipc = createMockIpc({ terminal: true });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e));
    await tick();
    await ipc.sshHostKeyDecision(id, false);
    expect(events.at(-1)).toMatchObject({ type: "error", error: { code: "auth_failed" } });
  });

  it("a changed key fails with host_key_mismatch until the stored key is removed", async () => {
    const ipc = createMockIpc({ terminal: true });
    const doc = await ipc.docGet();
    const base = doc.nodes.find((n) => n.id === MOCK_IDS.localSshd);
    if (!base || base.type !== "connection") throw new Error("seed changed");
    const changed = await ipc.docUpsertNode({ ...base, id: crypto.randomUUID(), name: "Changed", host: "changed.example", port: null });

    const events: TermEvent[] = [];
    await ipc.termOpen(changed.id, 80, 24, (e) => events.push(e));
    await tick();
    expect(events[0]).toMatchObject({ type: "error", error: { code: "host_key_mismatch" } });
    expect(events.some((e) => e.type === "hostKeyPrompt")).toBe(false);

    await ipc.knownHostsRemove("changed.example", 22);
    const after: TermEvent[] = [];
    await ipc.termOpen(changed.id, 80, 24, (e) => after.push(e));
    await tick();
    expect(after[0]).toMatchObject({ type: "hostKeyPrompt" });
  });

  it("reports unknown sessions and non-terminal connections with stable codes", async () => {
    const ipc = createMockIpc({ terminal: true });
    await expect(ipc.termWrite("nope", new Uint8Array())).rejects.toBeInstanceOf(IpcError);
    await expect(ipc.termOpen(MOCK_IDS.rdpServer, 80, 24, () => undefined)).rejects.toMatchObject({
      code: "invalid_input",
    });
    await expect(ipc.termOpen(crypto.randomUUID(), 80, 24, () => undefined)).rejects.toMatchObject({
      code: "not_found",
    });
  });

  it("closes terminal sessions; other session ids stay unsupported", async () => {
    const ipc = createMockIpc({ terminal: true });
    const id = await ipc.termOpen(MOCK_IDS.localZsh, 80, 24, () => undefined);
    await ipc.sessionClose(id);
    await expect(ipc.termWrite(id, new Uint8Array())).rejects.toMatchObject({ code: "not_found" });
    await expect(ipc.sessionClose("web-1")).rejects.toMatchObject({ code: "unsupported" });
  });
});
