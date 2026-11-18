import { describe, expect, it, vi } from "vitest";

import { SessionKeysMenu } from "./menuKeys";

function setup(fail = false) {
  const sent: boolean[] = [];
  let failNext = fail;
  const ipc = {
    menuSetSessionKeys: vi.fn(async (captured: boolean) => {
      sent.push(captured);
      if (failNext) {
        failNext = false;
        throw new Error("boom");
      }
    }),
    uiLog: vi.fn(async () => undefined),
  };
  return { menu: new SessionKeysMenu(ipc), sent, ipc };
}

describe("SessionKeysMenu", () => {
  it("sends only changes and keeps the menu out while any view holds the keyboard", () => {
    const { menu, sent } = setup();
    const a = {};
    const b = {};
    menu.set(a, false);
    menu.set(a, true);
    menu.set(b, true);
    menu.set(a, false);
    expect(sent).toEqual([true]);
    menu.set(b, false);
    expect(sent).toEqual([true, false]);
  });

  it("sends a held keyboard again when forced, but never a release", () => {
    const { menu, sent } = setup();
    const a = {};
    menu.set(a, true);
    menu.set(a, true, true);
    expect(sent).toEqual([true, true]);
    menu.set(a, false, true);
    expect(sent).toEqual([true, true, false]);
  });

  it("tries again after a failed call instead of believing it was sent", async () => {
    const { menu, sent, ipc } = setup(true);
    const a = {};
    menu.set(a, true);
    await vi.waitFor(() => expect(ipc.uiLog).toHaveBeenCalled());
    // The same state is asked again (for instance by the next focus change): it is sent again.
    menu.set(a, true);
    expect(sent).toEqual([true, true]);
  });
});
