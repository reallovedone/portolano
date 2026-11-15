import { describe, expect, it } from "vitest";

import { createMockIpc, MOCK_IDS } from "../../ipc/mock";

const bounds = { x: 1, y: 2, width: 300, height: 200 };

async function codeOf(p: Promise<unknown>): Promise<string> {
  try {
    await p;
  } catch (e) {
    return (e as { code: string }).code;
  }
  return "none";
}

describe("mock web sessions", () => {
  it("rejects with unsupported unless enabled", async () => {
    const ipc = createMockIpc();
    expect(await codeOf(ipc.webOpen(MOCK_IDS.publicSite, bounds))).toBe("unsupported");
  });

  it("opens a web connection, updates it and closes it", async () => {
    const ipc = createMockIpc({ web: true });
    const id = await ipc.webOpen(MOCK_IDS.publicSite, bounds);
    await ipc.webSetBounds(id, bounds, true);
    await ipc.webNavigate(id, { type: "url", url: "https://example.org" });
    await ipc.webNavigate(id, { type: "back" });
    await ipc.webNavigate(id, { type: "forward" });
    await ipc.webNavigate(id, { type: "reload" });
    await ipc.sessionClose(id);
    expect(await codeOf(ipc.webSetBounds(id, bounds, true))).toBe("not_found");
  });

  it("validates like the backend", async () => {
    const ipc = createMockIpc({ web: true });
    expect(await codeOf(ipc.webOpen(MOCK_IDS.lab, bounds))).toBe("invalid_input");
    expect(await codeOf(ipc.webOpen("00000000-0000-4000-8000-00000000ffff", bounds))).toBe("not_found");
    const id = await ipc.webOpen(MOCK_IDS.publicSite, bounds);
    expect(await codeOf(ipc.webNavigate(id, { type: "url", url: "file:///etc/passwd" }))).toBe("invalid_input");
    expect(await codeOf(ipc.webNavigate(id, { type: "url", url: "example.com" }))).toBe("invalid_input");
    expect(await codeOf(ipc.webSetBounds(id, { ...bounds, x: Number.NaN }, true))).toBe("invalid_input");
  });
});
