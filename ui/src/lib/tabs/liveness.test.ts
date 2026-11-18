import { describe, expect, it } from "vitest";

import type { SessionEvent } from "./session";
import { liveAfter } from "./liveness";

const data: SessionEvent = { type: "data", bytes: new Uint8Array([1]) };
const exit: SessionEvent = { type: "exit", code: 0 };
const error: SessionEvent = { type: "error", error: { code: "internal", message: "x" } };
const resize: SessionEvent = { type: "resize", w: 1, h: 1 };
const rect: SessionEvent = { type: "rect", x: 0, y: 0, w: 1, h: 1, rgba: new Uint8Array(4) };
const closed: SessionEvent = { type: "closed", reason: "bye" };
const credentialPrompt: SessionEvent = {
  type: "credentialPrompt",
  kind: "password",
  usernameHint: null,
  domainHint: null,
  target: "h",
  error: null,
  canSaveToVault: false,
};
const hostKey: SessionEvent = {
  type: "hostKeyPrompt",
  fingerprint: "f",
  algorithm: "a",
  host: "h",
  port: 22,
};
const cert: SessionEvent = { type: "certPrompt", fingerprint: "f", changed: false, stored: null };

describe("liveAfter", () => {
  it("never marks a web tab live", () => {
    for (const e of [data, exit, error, resize, rect, closed]) {
      expect(liveAfter("web", false, e)).toBe(false);
    }
  });

  it("connects a terminal at the first data and disconnects at exit or error", () => {
    for (const kind of ["ssh", "localShell"] as const) {
      expect(liveAfter(kind, false, data)).toBe(true);
      expect(liveAfter(kind, true, data)).toBe(true);
      expect(liveAfter(kind, true, exit)).toBe(false);
      expect(liveAfter(kind, true, error)).toBe(false);
      expect(liveAfter(kind, false, resize)).toBe(false);
    }
  });

  it("connects RDP and VNC at the first resize or rect and disconnects at closed or error", () => {
    for (const kind of ["rdp", "vnc"] as const) {
      expect(liveAfter(kind, false, resize)).toBe(true);
      expect(liveAfter(kind, false, rect)).toBe(true);
      expect(liveAfter(kind, true, closed)).toBe(false);
      expect(liveAfter(kind, true, error)).toBe(false);
      expect(liveAfter(kind, false, data)).toBe(false);
    }
  });

  it("ignores credential, certificate and host key prompts", () => {
    for (const live of [true, false]) {
      expect(liveAfter("ssh", live, credentialPrompt)).toBe(live);
      expect(liveAfter("ssh", live, hostKey)).toBe(live);
      expect(liveAfter("rdp", live, credentialPrompt)).toBe(live);
      expect(liveAfter("rdp", live, cert)).toBe(live);
    }
  });
});
