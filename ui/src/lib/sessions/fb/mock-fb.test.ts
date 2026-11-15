import { describe, expect, it, vi } from "vitest";

import { IpcError } from "../../ipc/errors";
import type { FbEvent, Node } from "../../ipc/types";

type ConnectionNode = Extract<Node, { type: "connection" }>;
import { MOCK_IDS, createMockIpc } from "../../ipc/mock";
import { testPattern } from "./mock-fb";

// An unlocked vault: the seeded connections then get their credential from it and these tests
// are about the certificate and the picture, not about the credential prompt.
async function open(options: { framebuffer?: boolean } = { framebuffer: true }) {
  const ipc = createMockIpc({ ...options, vault: "unlocked" });
  const events: FbEvent[] = [];
  const id = await ipc.fbOpen(MOCK_IDS.rdpServer, 800, 600, (e) => events.push(e));
  return { ipc, id, events };
}

const rects = (events: FbEvent[]) =>
  events.filter((e): e is Extract<FbEvent, { type: "rect" }> => e.type === "rect");

describe("mock framebuffer", () => {
  it("is off by default, like the other mock session families", async () => {
    const ipc = createMockIpc();
    await expect(ipc.fbOpen(MOCK_IDS.rdpServer, 800, 600, () => undefined)).rejects.toMatchObject({
      code: "unsupported",
    });
    await expect(ipc.fbKey("x", "KeyA", true)).rejects.toBeInstanceOf(IpcError);
  });

  it("asks for certificate trust once per host, then connects with the test pattern", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events[0]?.type).toBe("certPrompt"));
    await ipc.fbCertDecision(id, true);
    expect(events.map((e) => e.type)).toEqual(["certPrompt", "resize", "rect", "cursor"]);
    const first = rects(events)[0];
    expect(first).toMatchObject({ x: 0, y: 0, w: 800, h: 600 });
    expect(first?.rgba.length).toBe(800 * 600 * 4);
    expect(Array.from(first?.rgba.subarray(0, 4) ?? [])).toEqual([255, 255, 255, 255]);

    // The same host is trusted for the next session.
    const again: FbEvent[] = [];
    await ipc.fbOpen(MOCK_IDS.rdpServer, 400, 300, (e) => again.push(e));
    await vi.waitFor(() => expect(again.map((e) => e.type)).toEqual(["resize", "rect", "cursor"]));
  });

  it("closes the session when the certificate is declined", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, false);
    expect(events.at(-1)).toMatchObject({ type: "closed" });
  });

  it("echoes the pointer as a small square and restores the pattern behind it", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, true);
    events.length = 0;
    await ipc.fbPointer(id, 100, 100, 0, 0);
    expect(rects(events)).toHaveLength(1);
    expect(rects(events)[0]).toMatchObject({ x: 100, y: 100, w: 9, h: 9 });
    expect(Array.from(rects(events)[0]?.rgba.subarray(0, 4) ?? [])).toEqual([255, 255, 255, 255]);
    events.length = 0;
    await ipc.fbPointer(id, 300, 300, 1, 0);
    expect(rects(events).map((r) => [r.x, r.y])).toEqual([
      [100, 100],
      [300, 300],
    ]);
    // The square left behind is the pattern again.
    const pattern = testPattern(800, 600);
    const restored = rects(events)[0]?.rgba;
    expect(Array.from(restored?.subarray(0, 4) ?? [])).toEqual(Array.from(pattern.subarray((100 * 800 + 100) * 4, (100 * 800 + 100) * 4 + 4)));
  });

  it("draws a cell per key press", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, true);
    events.length = 0;
    await ipc.fbKey(id, "KeyA", true);
    await ipc.fbKey(id, "KeyA", false);
    expect(rects(events)).toHaveLength(1);
  });

  it("sends nothing while hidden and a full redraw when shown again", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, true);
    events.length = 0;
    await ipc.fbSetVisible(id, false);
    await ipc.fbPointer(id, 50, 50, 0, 0);
    expect(events).toHaveLength(0);
    await ipc.fbSetVisible(id, true);
    expect(rects(events)).toHaveLength(1);
    expect(rects(events)[0]).toMatchObject({ x: 0, y: 0, w: 800, h: 600 });
  });

  it("resizes (even width) and redraws", async () => {
    const { ipc, id, events } = await open();
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, true);
    events.length = 0;
    await ipc.fbResize(id, 1001, 700);
    expect(events.map((e) => e.type)).toEqual(["resize", "rect"]);
    expect(events[0]).toMatchObject({ w: 1000, h: 700 });
  });

  it("rejects unknown sessions and non-framebuffer connections, and closes through sessionClose", async () => {
    const { ipc, id } = await open();
    await expect(ipc.fbKey("nope", "KeyA", true)).rejects.toMatchObject({ code: "not_found" });
    await expect(ipc.fbOpen(MOCK_IDS.localSshd, 1, 1, () => undefined)).rejects.toMatchObject({
      code: "invalid_input",
    });
    await expect(ipc.fbOpen(MOCK_IDS.lab, 1, 1, () => undefined)).rejects.toMatchObject({
      code: "invalid_input",
    });
    await ipc.sessionClose(id);
    await expect(ipc.fbKey(id, "KeyA", true)).rejects.toMatchObject({ code: "not_found" });
  });

  it("uses the fixed size of the connection", async () => {
    const ipc = createMockIpc({ framebuffer: true });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.rdpFixed, 1500, 900, (e) => events.push(e));
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.fbCertDecision(id, true);
    expect(events.find((e) => e.type === "resize")).toMatchObject({ w: 1280, h: 800 });
  });
});

describe("mock framebuffer: VNC over TLS", () => {
  const tlsConnection = (id: string, extra: Partial<ConnectionNode> = {}): ConnectionNode => ({
    type: "connection",
    id,
    parentId: null,
    name: "TLS VNC",
    host: "vnc.tls.example",
    port: 15901,
    credential: { type: "none" },
    username: null,
    tags: [],
    notes: "",
    options: { type: "vnc", viewOnly: false },
    ...extra,
  });

  async function openTls(extra: Partial<ConnectionNode> = {}) {
    const ipc = createMockIpc({ framebuffer: true });
    const node = tlsConnection("00000000-0000-4000-8000-0000000000aa", extra);
    await ipc.docUpsertNode(node);
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(node.id, 800, 600, (e) => events.push(e));
    return { ipc, id, events, node };
  }

  it("shows the certificate first, then asks user name and password, then reports TLS", async () => {
    const { ipc, id, events } = await openTls();
    await vi.waitFor(() => expect(events.map((e) => e.type)).toEqual(["certPrompt"]));
    // Nothing is asked before the certificate is trusted.
    await ipc.fbCertDecision(id, true);
    expect(events.map((e) => e.type)).toEqual(["certPrompt", "credentialPrompt"]);
    expect(events[1]).toMatchObject({ type: "credentialPrompt", kind: "usernamePassword" });
    await ipc.sessionCredential(
      id,
      { username: "tester", password: "ok-password", keyPath: null, passphrase: null, domain: null },
      false,
    );
    expect(events.map((e) => e.type)).toEqual([
      "certPrompt",
      "credentialPrompt",
      "security",
      "resize",
      "rect",
      "cursor",
    ]);
    expect(events.find((e) => e.type === "security")).toEqual({ type: "security", encrypted: true });
  });

  it("is also selected by the tls tag, and declining closes the session", async () => {
    const { ipc, id, events } = await openTls({ port: 5900, tags: ["TLS"] });
    await vi.waitFor(() => expect(events[0]?.type).toBe("certPrompt"));
    await ipc.fbCertDecision(id, false);
    expect(events.at(-1)).toMatchObject({ type: "closed" });
    expect(events.some((e) => e.type === "credentialPrompt" || e.type === "security")).toBe(false);
  });

  it("asks about the certificate only once per host", async () => {
    const { ipc, id, node } = await openTls();
    await ipc.fbCertDecision(id, true);
    const again: FbEvent[] = [];
    await ipc.fbOpen(node.id, 800, 600, (e) => again.push(e));
    await vi.waitFor(() => expect(again[0]?.type).toBe("credentialPrompt"));
    expect(again.some((e) => e.type === "certPrompt")).toBe(false);
  });

  it("leaves the default VNC path without certificate and reports it as not encrypted", async () => {
    const ipc = createMockIpc({ framebuffer: true, vault: "unlocked" });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.devVm, 800, 600, (e) => events.push(e));
    // The user agrees to the unencrypted password exchange first (asked once per host).
    await vi.waitFor(() => expect(events.map((e) => e.type)).toEqual(["unencryptedPrompt"]));
    await ipc.fbUnencryptedDecision(id, true);
    await vi.waitFor(() =>
      expect(events.map((e) => e.type)).toEqual(["unencryptedPrompt", "security", "resize", "rect", "cursor"]),
    );
    expect(events[1]).toEqual({ type: "security", encrypted: false });

    // The same host again: already agreed, no question.
    const again: FbEvent[] = [];
    await ipc.fbOpen(MOCK_IDS.devVm, 800, 600, (e) => again.push(e));
    await vi.waitFor(() => expect(again.map((e) => e.type)).toEqual(["security", "resize", "rect", "cursor"]));
  });

  it("declining the unencrypted exchange closes the session cleanly", async () => {
    const ipc = createMockIpc({ framebuffer: true, vault: "unlocked" });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.devVm, 800, 600, (e) => events.push(e));
    await vi.waitFor(() => expect(events.map((e) => e.type)).toEqual(["unencryptedPrompt"]));
    await ipc.fbUnencryptedDecision(id, false);
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe("closed"));
    expect(events.some((e) => e.type === "security" || e.type === "resize")).toBe(false);
  });
});
