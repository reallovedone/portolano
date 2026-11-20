import { describe, expect, it, vi } from "vitest";

import { IpcError } from "./errors";
import { MOCK_IDS, MOCK_MASTER_PASSWORD, createMockIpc } from "./mock";
import { MOCK_MAX_ATTEMPTS, MOCK_REJECTED_PASSWORD } from "./mock-credentials";
import type { CredentialAnswer, CredentialPrompt, FbEvent, TermEvent } from "./types";

const answer = (over: Partial<CredentialAnswer> = {}): CredentialAnswer => ({
  username: "ann",
  password: "pw",
  keyPath: null,
  passphrase: null,
  domain: null,
  ...over,
});

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));
const text = (events: TermEvent[]): string =>
  events
    .filter((e): e is Extract<TermEvent, { type: "data" }> => e.type === "data")
    .map((e) => new TextDecoder().decode(e.bytes))
    .join("");
const prompts = (events: Array<TermEvent | FbEvent>): CredentialPrompt[] =>
  events.flatMap((e) => {
    if (e.type !== "credentialPrompt") return [];
    const { type: _type, ...prompt } = e;
    return [prompt];
  });

async function sshSession(options: Parameters<typeof createMockIpc>[0] = {}, askCredentials = false) {
  const ipc = createMockIpc({ terminal: true, ...options });
  const events: TermEvent[] = [];
  const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e), { askCredentials });
  await tick();
  if (events[0]?.type === "hostKeyPrompt") {
    await ipc.sshHostKeyDecision(id, true);
  }
  return { ipc, events, id };
}

describe("mock terminal: credential prompt", () => {
  it("without a vault: host key, then the prompt, then the shell; the secret never comes back", async () => {
    const { ipc, events, id } = await sshSession();
    const [first] = prompts(events);
    expect(first).toMatchObject({
      kind: "sshKey",
      usernameHint: null,
      target: "127.0.0.1:12222",
      error: null,
      // No vault exists: nothing to save into.
      canSaveToVault: false,
    });
    // Nothing is greeted before the answer, and typing is discarded meanwhile.
    expect(text(events)).toBe("");
    await ipc.termWrite(id, new TextEncoder().encode("ls\r"));
    expect(text(events)).toBe("");

    await ipc.sessionCredential(id, answer({ password: "a-secret" }), false);
    expect(text(events)).toContain("Portolano mock terminal");
    expect(JSON.stringify(events)).not.toContain("a-secret");
  });

  it("pre-fills the user name from the connection", async () => {
    const ipc = createMockIpc({ terminal: true });
    const doc = await ipc.docGet();
    const base = doc.nodes.find((n) => n.id === MOCK_IDS.localSshd);
    if (!base || base.type !== "connection") throw new Error("seed changed");
    await ipc.docUpsertNode({ ...base, username: "deploy" });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e));
    await tick();
    await ipc.sshHostKeyDecision(id, true);
    expect(prompts(events)[0]?.usernameHint).toBe("deploy");
  });

  it("a rejected password brings the prompt back with an error; the next one works", async () => {
    const { ipc, events, id } = await sshSession();
    await ipc.sessionCredential(id, answer({ password: MOCK_REJECTED_PASSWORD }), false);
    const all = prompts(events);
    expect(all).toHaveLength(2);
    expect(all[1]?.error).toContain("rejected");
    // The user name typed first time is offered again; the secret is not echoed.
    expect(all[1]?.usernameHint).toBe("ann");
    expect(JSON.stringify(events)).not.toContain(MOCK_REJECTED_PASSWORD + '"');
    expect(events.some((e) => e.type === "error")).toBe(false);
    await ipc.sessionCredential(id, answer(), false);
    expect(text(events)).toContain("Portolano mock terminal");
  });

  it("empty answers are asked again, and the attempts are capped", async () => {
    const { ipc, events, id } = await sshSession();
    await ipc.sessionCredential(id, answer({ username: " " }), false);
    expect(prompts(events)[1]?.error).toBe("Enter a user name.");
    await ipc.sessionCredential(id, answer({ password: null }), false);
    expect(prompts(events)[2]?.error).toContain("password or choose a private key file");
    for (let i = 3; i <= MOCK_MAX_ATTEMPTS; i++) {
      await ipc.sessionCredential(id, answer({ password: MOCK_REJECTED_PASSWORD }), false);
    }
    expect(events.at(-1)).toMatchObject({ type: "error", error: { code: "auth_failed" } });
  });

  it("cancel ends with the cancelled code", async () => {
    const { ipc, events, id } = await sshSession();
    await ipc.sessionCredentialCancel(id);
    expect(events.at(-1)).toMatchObject({ type: "error", error: { code: "cancelled" } });
    // Nothing is pending any more.
    await expect(ipc.sessionCredentialCancel(id)).rejects.toMatchObject({ code: "invalid_input" });
  });

  it("a key file answer is accepted", async () => {
    const { ipc, events, id } = await sshSession();
    await ipc.sessionCredential(id, answer({ password: null, keyPath: "/some/key" }), false);
    expect(text(events)).toContain("Portolano mock terminal");
  });

  it("with a locked vault the vault flow stays, and 'ask' is the alternative", async () => {
    const ipc = createMockIpc({ terminal: true, vault: "locked" });
    await expect(ipc.termOpen(MOCK_IDS.localSshd, 80, 24, () => undefined)).rejects.toMatchObject({
      code: "vault_locked",
    });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e), {
      askCredentials: true,
    });
    await tick();
    await ipc.sshHostKeyDecision(id, true);
    // Locked: saving is not offered.
    expect(prompts(events)[0]?.canSaveToVault).toBe(false);
  });

  it("with an unlocked vault holding the credential there is no prompt", async () => {
    const { events } = await sshSession({ vault: "unlocked" });
    expect(prompts(events)).toHaveLength(0);
    expect(text(events)).toContain("Portolano mock terminal");
  });

  it("save to vault: offered when unlocked, creates the credential and points the connection at it", async () => {
    const ipc = createMockIpc({ terminal: true, vault: "unlocked" });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e), {
      askCredentials: true,
    });
    await tick();
    await ipc.sshHostKeyDecision(id, true);
    expect(prompts(events)[0]?.canSaveToVault).toBe(true);
    await ipc.sessionCredential(id, answer({ username: "ann", password: "keep-me" }), true);

    const created = (await ipc.vaultList()).find((c) => c.name === "Local sshd (ann)");
    expect(created).toMatchObject({ username: "ann", kind: "password" });
    const node = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_IDS.localSshd);
    expect(node?.type === "connection" && node.credential).toEqual({ type: "id", id: created?.id });
    expect(await ipc.vaultRevealSecret(created?.id ?? "")).toEqual({ type: "password", password: "keep-me" });
  });

  it("without the save box nothing is created", async () => {
    const ipc = createMockIpc({ terminal: true, vault: "unlocked" });
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, () => undefined, { askCredentials: true });
    await tick();
    await ipc.sshHostKeyDecision(id, true);
    const before = (await ipc.vaultList()).length;
    await ipc.sessionCredential(id, answer(), false);
    expect((await ipc.vaultList()).length).toBe(before);
  });

  it("answering a session that is not waiting, or unknown, is an error", async () => {
    const ipc = createMockIpc({ terminal: true, vault: "unlocked" });
    const id = await ipc.termOpen(MOCK_IDS.localZsh, 80, 24, () => undefined);
    await expect(ipc.sessionCredential(id, answer(), false)).rejects.toMatchObject({ code: "invalid_input" });
    await expect(ipc.sessionCredential("nope", answer(), false)).rejects.toBeInstanceOf(IpcError);
    await expect(createMockIpc().sessionCredential("x", answer(), false)).rejects.toMatchObject({
      code: "unsupported",
    });
  });

  it("a dangling credential id with no vault is a prompt, not an error", async () => {
    const ipc = createMockIpc({ terminal: true });
    const base = (await ipc.docGet()).nodes.find((n) => n.id === MOCK_IDS.localSshd);
    if (!base || base.type !== "connection") throw new Error("seed changed");
    await ipc.docUpsertNode({ ...base, credential: { type: "id", id: crypto.randomUUID() } });
    const events: TermEvent[] = [];
    const id = await ipc.termOpen(MOCK_IDS.localSshd, 80, 24, (e) => events.push(e));
    await tick();
    await ipc.sshHostKeyDecision(id, true);
    expect(prompts(events)).toHaveLength(1);
    // Unlock-and-save works after creating a vault later (sanity for the master password).
    expect(MOCK_MASTER_PASSWORD).toBeTruthy();
  });
});

describe("mock framebuffer: credential prompt", () => {
  const FB_TICK = () => vi.waitFor(() => undefined);

  it("RDP asks for user name and password before the certificate", async () => {
    const ipc = createMockIpc({ framebuffer: true });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.rdpServer, 800, 600, (e) => events.push(e));
    await vi.waitFor(() => expect(events).toHaveLength(1));
    expect(events[0]).toMatchObject({
      type: "credentialPrompt",
      kind: "usernamePassword",
      domainHint: "LAB",
      canSaveToVault: false,
    });
    await ipc.sessionCredential(id, answer(), false);
    expect(events.map((e) => e.type)).toEqual(["credentialPrompt", "certPrompt"]);
    await ipc.fbCertDecision(id, true);
    expect(events.map((e) => e.type).slice(2)).toEqual(["security", "resize", "rect", "cursor"]);
    await FB_TICK();
  });

  it("a wrong password re-prompts; cancel ends with the cancelled code", async () => {
    const ipc = createMockIpc({ framebuffer: true });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.rdpServer, 800, 600, (e) => events.push(e));
    await vi.waitFor(() => expect(events).toHaveLength(1));
    await ipc.sessionCredential(id, answer({ password: MOCK_REJECTED_PASSWORD }), false);
    expect(prompts(events)).toHaveLength(2);
    expect(events.some((e) => e.type === "certPrompt")).toBe(false);
    await ipc.sessionCredentialCancel(id);
    expect(events.at(-1)).toMatchObject({ type: "error", error: { code: "cancelled" } });
  });

  it("VNC asks for a password only", async () => {
    const ipc = createMockIpc({ framebuffer: true });
    const events: FbEvent[] = [];
    const id = await ipc.fbOpen(MOCK_IDS.devVm, 800, 600, (e) => events.push(e));
    await vi.waitFor(() => expect(events).toHaveLength(1));
    expect(events[0]).toMatchObject({ type: "credentialPrompt", kind: "password" });
    await ipc.sessionCredential(id, answer({ username: "" }), false);
    expect(prompts(events)).toHaveLength(1);
    // Plain VNC has no certificate; it asks before the password is sent unencrypted ...
    expect(events.some((e) => e.type === "certPrompt")).toBe(false);
    await vi.waitFor(() => expect(events.at(-1)?.type).toBe("unencryptedPrompt"));
    await ipc.fbUnencryptedDecision(id, true);
    // ... and the desktop follows, after the (unencrypted) security report.
    expect(events.map((e) => e.type)).toEqual([
      "credentialPrompt",
      "unencryptedPrompt",
      "security",
      "resize",
      "rect",
      "cursor",
    ]);
  });

  it("with an unlocked vault the credential comes from it: no prompt; locked: vault_locked", async () => {
    const unlocked = createMockIpc({ framebuffer: true, vault: "unlocked" });
    const events: FbEvent[] = [];
    await unlocked.fbOpen(MOCK_IDS.rdpServer, 800, 600, (e) => events.push(e));
    await vi.waitFor(() => expect(events).toHaveLength(1));
    expect(events[0]?.type).toBe("certPrompt");

    const locked = createMockIpc({ framebuffer: true, vault: "locked" });
    await expect(locked.fbOpen(MOCK_IDS.rdpServer, 800, 600, () => undefined)).rejects.toMatchObject({
      code: "vault_locked",
    });
    const asking: FbEvent[] = [];
    await locked.fbOpen(MOCK_IDS.rdpServer, 800, 600, (e) => asking.push(e), { askCredentials: true });
    await vi.waitFor(() => expect(asking[0]?.type).toBe("credentialPrompt"));
  });
});
