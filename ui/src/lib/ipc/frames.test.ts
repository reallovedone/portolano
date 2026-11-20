import { describe, expect, it } from "vitest";

import { decodeFbEvent, decodeTermEvent, toBytes } from "./frames";

const bytes = (...n: number[]) => Uint8Array.from(n);
const json = (tag: number, value: unknown) =>
  Uint8Array.from([tag, ...new TextEncoder().encode(JSON.stringify(value))]);
const raw = (tag: number, text: string) => Uint8Array.from([tag, ...new TextEncoder().encode(text)]);

// The binary vectors below are the same ones asserted in the Rust tests
// (crates/core-model/src/fb.rs and crates/session-term/src/lib.rs).
describe("decodeFbEvent clipboard", () => {
  // The same bytes are asserted in crates/core-model/src/fb.rs (clipboard_frame_layout).
  it("decodes the four clipboard events", () => {
    expect(
      decodeFbEvent(raw(9, '{"type":"remoteFiles","count":2,"bytes":1500,"skipped":1}')),
    ).toEqual({ type: "clipboard", event: { type: "remoteFiles", count: 2, bytes: 1500, skipped: 1 } });
    expect(
      decodeFbEvent(
        raw(9, '{"type":"progress","filesDone":1,"filesTotal":3,"bytesDone":10,"bytesTotal":30}'),
      ),
    ).toEqual({
      type: "clipboard",
      event: { type: "progress", filesDone: 1, filesTotal: 3, bytesDone: 10, bytesTotal: 30 },
    });
    expect(decodeFbEvent(raw(9, '{"type":"done","files":3,"bytes":30,"skipped":0}'))).toEqual({
      type: "clipboard",
      event: { type: "done", files: 3, bytes: 30, skipped: 0 },
    });
    expect(decodeFbEvent(raw(9, '{"type":"failed","reason":"cancelled"}'))).toEqual({
      type: "clipboard",
      event: { type: "failed", reason: "cancelled" },
    });
  });

  it("refuses a malformed clipboard frame instead of guessing", () => {
    expect(() => decodeFbEvent(raw(9, '{"type":"failed","reason":"mystery"}'))).toThrow();
    expect(() => decodeFbEvent(raw(9, '{"type":"other"}'))).toThrow();
    expect(() => decodeFbEvent(raw(9, "not json"))).toThrow();
    // Numbers that are not plain counts become 0.
    expect(
      decodeFbEvent(raw(9, '{"type":"remoteFiles","count":-3,"bytes":"x","skipped":null}')),
    ).toEqual({ type: "clipboard", event: { type: "remoteFiles", count: 0, bytes: 0, skipped: 0 } });
  });
});

describe("decodeFbEvent", () => {
  it("decodes resize", () => {
    expect(decodeFbEvent(bytes(0, 0x02, 0x01, 0x04, 0x03))).toEqual({
      type: "resize",
      w: 0x0102,
      h: 0x0304,
    });
  });

  it("decodes rect with its pixels", () => {
    expect(decodeFbEvent(bytes(1, 1, 0, 2, 0, 1, 0, 1, 0, 9, 8, 7, 6))).toEqual({
      type: "rect",
      x: 1,
      y: 2,
      w: 1,
      h: 1,
      rgba: bytes(9, 8, 7, 6),
    });
  });

  it("decodes cursor", () => {
    expect(decodeFbEvent(bytes(2, 3, 0, 4, 0, 1, 0, 1, 0, 1, 2, 3, 4))).toEqual({
      type: "cursor",
      hotspotX: 3,
      hotspotY: 4,
      w: 1,
      h: 1,
      rgba: bytes(1, 2, 3, 4),
    });
  });

  it("decodes json events", () => {
    expect(decodeFbEvent(json(3, { fingerprint: "ab", changed: true, stored: "cd" }))).toEqual({
      type: "certPrompt",
      fingerprint: "ab",
      changed: true,
      stored: "cd",
    });
    // Frames of an older backend have no `stored`.
    expect(decodeFbEvent(json(3, { fingerprint: "ab", changed: false }))).toEqual({
      type: "certPrompt",
      fingerprint: "ab",
      changed: false,
      stored: null,
    });
    expect(decodeFbEvent(raw(3, '{"changed":false,"fingerprint":"ab:cd","stored":null}'))).toEqual({
      type: "certPrompt",
      fingerprint: "ab:cd",
      changed: false,
      stored: null,
    });
    // The exact bytes asserted in core-model fb.rs.
    expect(decodeFbEvent(raw(8, "{}"))).toEqual({ type: "unencryptedPrompt" });
    expect(decodeFbEvent(json(4, { reason: "bye" }))).toEqual({ type: "closed", reason: "bye" });
    expect(decodeFbEvent(json(5, { code: "io", message: "boom" }))).toEqual({
      type: "error",
      error: { code: "io", message: "boom" },
    });
  });

  it("decodes the credential prompt (tag 6), the exact bytes asserted in core-model fb.rs", () => {
    const frame = raw(
      6,
      '{"kind":"password","usernameHint":null,"domainHint":null,"target":"vnc.example:5900","error":"wrong","canSaveToVault":false}',
    );
    expect(decodeFbEvent(frame)).toEqual({
      type: "credentialPrompt",
      kind: "password",
      usernameHint: null,
      domainHint: null,
      target: "vnc.example:5900",
      error: "wrong",
      canSaveToVault: false,
    });
  });

  it("decodes the security report (tag 7), the exact bytes asserted in core-model fb.rs", () => {
    expect(decodeFbEvent(raw(7, '{"encrypted":true}'))).toEqual({ type: "security", encrypted: true });
    expect(decodeFbEvent(raw(7, '{"encrypted":false}'))).toEqual({ type: "security", encrypted: false });
    // RDP adds how the login was authenticated; the exact bytes are asserted in core-model fb.rs.
    for (const auth of ["negotiate", "ntlm", "none"] as const) {
      expect(decodeFbEvent(raw(7, `{"encrypted":true,"auth":"${auth}"}`))).toEqual({
        type: "security",
        encrypted: true,
        auth,
      });
    }
    // An unknown mechanism is dropped, not passed on to the UI.
    expect(decodeFbEvent(raw(7, '{"encrypted":true,"auth":"x"}'))).toEqual({ type: "security", encrypted: true });
    // A frame without the flag is never read as "encrypted".
    expect(decodeFbEvent(raw(7, "{}"))).toEqual({ type: "security", encrypted: false });
  });

  it("rejects truncated and unknown frames", () => {
    expect(() => decodeFbEvent(bytes())).toThrow();
    expect(() => decodeFbEvent(bytes(1, 0, 0))).toThrow();
    expect(() => decodeFbEvent(bytes(99))).toThrow();
  });
});

describe("decodeTermEvent", () => {
  it("decodes data as the raw bytes after the tag", () => {
    expect(decodeTermEvent(bytes(0, 1, 2, 3))).toEqual({ type: "data", bytes: bytes(1, 2, 3) });
  });

  it("decodes json events", () => {
    expect(
      decodeTermEvent(
        json(1, { fingerprint: "SHA256:x", algorithm: "ssh-ed25519", host: "jump.example", port: 2222 }),
      ),
    ).toEqual({
      type: "hostKeyPrompt",
      fingerprint: "SHA256:x",
      algorithm: "ssh-ed25519",
      host: "jump.example",
      port: 2222,
    });
    // The exact bytes asserted in session-term lib.rs (host and port are additive fields).
    expect(
      decodeTermEvent(
        raw(1, '{"algorithm":"ssh-ed25519","fingerprint":"SHA256:x","host":"jump.example","port":2222}'),
      ),
    ).toMatchObject({ host: "jump.example", port: 2222 });
    expect(decodeTermEvent(json(2, { code: 3 }))).toEqual({ type: "exit", code: 3 });
    expect(decodeTermEvent(json(2, { code: null }))).toEqual({ type: "exit", code: null });
    expect(decodeTermEvent(json(3, { code: "host_key_mismatch", message: "m" }))).toEqual({
      type: "error",
      error: { code: "host_key_mismatch", message: "m" },
    });
  });

  it("decodes the credential prompt (tag 4), the exact bytes asserted in session-term lib.rs", () => {
    const frame = raw(
      4,
      '{"kind":"sshKey","usernameHint":"tester","domainHint":null,"target":"h:22","error":null,"canSaveToVault":false}',
    );
    expect(decodeTermEvent(frame)).toEqual({
      type: "credentialPrompt",
      kind: "sshKey",
      usernameHint: "tester",
      domainHint: null,
      target: "h:22",
      error: null,
      canSaveToVault: false,
    });
    // Optional fields may be missing; a save offer needs an explicit `true`.
    expect(
      decodeTermEvent(json(4, { kind: "usernamePassword", target: "t", canSaveToVault: true })),
    ).toMatchObject({ usernameHint: null, domainHint: null, error: null, canSaveToVault: true });
    expect(() => decodeTermEvent(json(4, { kind: "nonsense", target: "t" }))).toThrow();
    expect(() => decodeTermEvent(json(4, { kind: "password" }))).toThrow();
  });

  it("decodes the cancelled error code", () => {
    expect(decodeTermEvent(json(3, { code: "cancelled", message: "m" }))).toEqual({
      type: "error",
      error: { code: "cancelled", message: "m" },
    });
  });

  it("rejects malformed frames", () => {
    expect(() => decodeTermEvent(bytes(1, 0x7b))).toThrow();
    expect(() => decodeTermEvent(bytes(42))).toThrow();
  });
});

describe("toBytes", () => {
  it("accepts ArrayBuffer, views and number arrays", () => {
    expect(toBytes(bytes(1, 2).buffer as ArrayBuffer)).toEqual(bytes(1, 2));
    expect(toBytes(new DataView(bytes(3, 4).buffer))).toEqual(bytes(3, 4));
    expect(toBytes([5, 6])).toEqual(bytes(5, 6));
  });
});
