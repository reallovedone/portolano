import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../../App.svelte";
import { MOCK_MASTER_PASSWORD, createMockIpc, type MockIpc } from "../ipc/mock";
import { MOCK_SHARE_IDS } from "../ipc/mock-shares";
import { createApp, type App as AppState } from "../state/app";
import { createMockFilePicker } from "../transfer/files";
import { treeSelectOptions } from "../ui/treeSelectTesting";
import { checkLocation, displayText, hostLine } from "./display";

vi.mock("@xterm/xterm", async () => ({
  Terminal: (await import("../sessions/term/fakeXterm")).FakeTerminal,
}));

async function start(
  options: Parameters<typeof createMockIpc>[0] = {},
  files = createMockFilePicker(),
): Promise<{ app: AppState; ipc: MockIpc }> {
  const ipc = createMockIpc({ vault: "unlocked", ...options });
  const app = createApp(ipc, files);
  render(App, { props: { app, demo: false } });
  await waitFor(() => expect(app.doc.status).toBe("ready"));
  return { app, ipc };
}

async function lockVault(app: AppState): Promise<void> {
  await app.vault.lock();
  expect(app.vault.unlocked).toBe(false);
}

async function cancelUnlock(): Promise<void> {
  const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
  await fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull());
}

async function unlockWithPassword(): Promise<void> {
  const dialog = await screen.findByRole("dialog", { name: "Unlock the vault" });
  await fireEvent.input(within(dialog).getByLabelText("Master password"), {
    target: { value: MOCK_MASTER_PASSWORD },
  });
  await fireEvent.click(within(dialog).getByRole("button", { name: "Unlock" }));
  await waitFor(() => expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull());
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  window.localStorage.clear();
});

describe("display helpers", () => {
  it("strips_bidi_controls_from_untrusted_names", () => {
    expect(displayText("Ada\u202Egnp.exe\u2066\u2069")).toBe("Adagnp.exe");
    expect(displayText("a\u200Eb\u200Fc\u061Cd")).toBe("abcd");
    expect(displayText("line\nbreak")).toBe("line break");
    expect(displayText(null)).toBe("");
  });

  it("caps_long_untrusted_text_on_a_code_point_boundary", () => {
    const capped = displayText("a".repeat(5000));
    expect(Array.from(capped)).toHaveLength(81);
    expect(capped.endsWith("…")).toBe(true);
    expect(displayText("a".repeat(5000), 300)).toHaveLength(301);
    expect(displayText("short")).toBe("short");
    expect(displayText("😀".repeat(200), 10)).toBe(`${"😀".repeat(10)}…`);
  });

  it("caps_text_on_a_grapheme_boundary", () => {
    const family = "👨‍👩‍👧";
    expect(displayText(family.repeat(20), 5)).toBe(`${family.repeat(5)}…`);
    expect(displayText("e\u0301".repeat(20), 3)).toBe(`${"e\u0301".repeat(3)}…`);
  });

  it("shows_a_web_connection_by_its_address_and_a_local_shell_by_its_kind", () => {
    const base = { nodeId: "n", name: "Wiki", protocol: "web", host: "https://wiki.example/x", port: 8443, username: "ada" };
    expect(hostLine(base)).toBe("Wiki: web https://wiki.example/x");
    expect(hostLine({ ...base, name: "Shell", protocol: "local", host: "", port: null, username: null })).toBe(
      "Shell: local",
    );
  });

  it("checks_the_location_field", () => {
    expect(checkLocation("   ")).toEqual({ ok: false, problem: "empty" });
    expect(checkLocation("http://a.example/x")).toEqual({ ok: false, problem: "notHttps" });
    expect(checkLocation("ftp://a.example/x")).toEqual({ ok: false, problem: "notHttps" });
    expect(checkLocation("https://user:pw@a.example/x")).toEqual({ ok: false, problem: "credentials" });
    expect(checkLocation("https://user@a.example/x")).toEqual({ ok: false, problem: "credentials" });
    expect(checkLocation("https://a.example/x")).toEqual({
      ok: true,
      location: { type: "url", value: "https://a.example/x" },
    });
    expect(checkLocation("/Volumes/Team/lab.portolano-share")).toEqual({
      ok: true,
      location: { type: "path", value: "/Volumes/Team/lab.portolano-share" },
    });
    expect(checkLocation("C:\\Share\\lab.portolano-share").ok).toBe(true);
  });

  it("checks_the_location_field_edge_cases", () => {
    expect(checkLocation("\\\\server\\share\\a@b.portolano-share").ok).toBe(true);
    expect(checkLocation("/Volumes/me@work/lab.portolano-share").ok).toBe(true);
    expect(checkLocation("file:///x")).toEqual({ ok: false, problem: "file" });
    expect(checkLocation("HTTPS://a.example/x").ok).toBe(true);
    expect(checkLocation("https://host\\@evil/x").ok).toBe(false);
    expect(checkLocation("https:///")).toEqual({ ok: false, problem: "invalid" });
  });
});

describe("AddRemoteFolderDialog", () => {
  it("add_remote_folder_shows_fingerprint_before_subscribing", async () => {
    const { app, ipc } = await start();
    const subscribe = vi.spyOn(ipc, "shareSubscribe");
    app.ui.open({ type: "addRemoteFolder" });
    await screen.findByRole("dialog", { name: "Add a remote folder" });
    await fireEvent.input(screen.getByLabelText("Share file or address"), {
      target: { value: "/Volumes/Team/lab.portolano-share" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(await screen.findByText("7F3A-91C2-0D4E-B6A8")).toBeTruthy();
    expect(screen.getByText("Check this fingerprint with the person who shared the folder.")).toBeTruthy();
    expect(subscribe).not.toHaveBeenCalled();
    expect(app.doc.shares).toHaveLength(0);

    await fireEvent.click(screen.getByRole("button", { name: "Subscribe" }));
    await waitFor(() => expect(app.ui.dialog).toBeNull());
    expect(subscribe).toHaveBeenCalledTimes(1);
    expect(app.doc.shares).toHaveLength(1);
    expect(app.doc.shares[0]?.role).toBe("subscriber");
  });

  async function previewShare(app: AppState): Promise<void> {
    app.ui.open({ type: "addRemoteFolder" });
    await screen.findByRole("dialog", { name: "Add a remote folder" });
    await fireEvent.input(screen.getByLabelText("Share file or address"), {
      target: { value: "/Volumes/Team/lab.portolano-share" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    await screen.findByText("7F3A-91C2-0D4E-B6A8");
  }

  it("add_remote_folder_lists_the_hosts_before_subscribing", async () => {
    const { app } = await start();
    await previewShare(app);
    expect(screen.getByText("Gateway: ssh ops@gw.team.example")).toBeTruthy();
    expect(screen.getByText("Desk: rdp 10.0.0.5:3390")).toBeTruthy();
  });

  it("add_remote_folder_shows_a_long_host_in_full", async () => {
    const { app, ipc } = await start();
    const base = await ipc.sharePreview({ type: "path", value: "/Volumes/Team/lab.portolano-share" });
    const host = `corp-dc.internal.${"p".repeat(170)}.evil.io`;
    expect(host.length).toBeGreaterThan(190);
    vi.spyOn(ipc, "sharePreview").mockResolvedValue({ ...base, hosts: [{ ...base.hosts[0]!, host }] });
    await previewShare(app);
    expect(screen.getByText(new RegExp(`${host.replace(/\./g, "\\.")}`))).toBeTruthy();
  });

  it("add_remote_folder_names_the_left_out_connections_and_counts_the_rest", async () => {
    const { app, ipc } = await start();
    const base = await ipc.sharePreview({ type: "path", value: "/Volumes/Team/lab.portolano-share" });
    const skipped = Array.from({ length: 8 }, (_, i) => `Skipped ${i}`);
    vi.spyOn(ipc, "sharePreview").mockResolvedValue({ ...base, skipped });
    await previewShare(app);
    expect(screen.getByText(/Left out \(8\): Skipped 0, Skipped 1, Skipped 2, Skipped 3, Skipped 4 and 3 more/)).toBeTruthy();
  });

  it("add_remote_folder_never_truncates_a_long_host_list", async () => {
    const { app, ipc } = await start();
    const base = await ipc.sharePreview({ type: "path", value: "/Volumes/Team/lab.portolano-share" });
    const hosts = Array.from({ length: 60 }, (_, i) => ({
      ...base.hosts[0]!,
      nodeId: `host-${i}`,
      name: `Host ${i}`,
    }));
    vi.spyOn(ipc, "sharePreview").mockResolvedValue({ ...base, hosts });
    await previewShare(app);
    const list = screen.getAllByRole("list").find((l) => l.classList.contains("hosts"))!;
    expect(within(list).getAllByRole("listitem")).toHaveLength(60);
  });

  it("add_remote_folder_warns_only_when_the_destination_has_a_credential", async () => {
    const warning = /will use the credential of “Lab” unless you set one/;
    const { app } = await start();
    app.ui.selectedId = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!.id;
    await previewShare(app);
    expect(screen.getByText(warning)).toBeTruthy();
    expect(screen.getByText(/Check the hosts above\./)).toBeTruthy();
    cleanup();

    const second = await start();
    second.app.ui.selectedId = null;
    await previewShare(second.app);
    expect(screen.queryByText(/will use the credential of/)).toBeNull();
  });

  it("add_remote_folder_refuses_http_url", async () => {
    const { app, ipc } = await start();
    const preview = vi.spyOn(ipc, "sharePreview");
    app.ui.open({ type: "addRemoteFolder" });
    await screen.findByRole("dialog", { name: "Add a remote folder" });
    const field = screen.getByLabelText("Share file or address");

    await fireEvent.input(field, { target: { value: "http://shares.example/team.portolano-share" } });
    await fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(await screen.findByText(/Only https:\/\/ addresses are accepted/)).toBeTruthy();

    await fireEvent.input(field, { target: { value: "https://ada:secret@shares.example/team.portolano-share" } });
    await fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    expect(await screen.findByText(/must not contain a user name or password/)).toBeTruthy();

    expect(preview).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Subscribe" })).toBeNull();
  });

  it("add_remote_folder_never_offers_a_shared_folder_as_destination", async () => {
    const { app } = await start({ shares: true });
    app.ui.selectedId = MOCK_SHARE_IDS.servers;
    app.ui.open({ type: "addRemoteFolder" });
    await screen.findByRole("dialog", { name: "Add a remote folder" });
    await fireEvent.input(screen.getByLabelText("Share file or address"), {
      target: { value: "/Volumes/Team/lab.portolano-share" },
    });
    await fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    await screen.findByText("7F3A-91C2-0D4E-B6A8");
    const options = await treeSelectOptions(screen.getByRole("combobox", { name: /Add under/ }));
    const remoteIds = new Set(app.doc.shares.flatMap((x) => [x.rootNodeId]));
    expect(remoteIds.size).toBeGreaterThan(0);
    const remoteNames = app.doc.nodes.filter((n) => remoteIds.has(n.id)).map((n) => n.name);
    for (const name of remoteNames) expect(options.map((o) => o.label)).not.toContain(name);
    expect(options.length).toBeGreaterThan(1);
    // The selected node is inside a shared folder: the default falls back to the top level.
    await fireEvent.click(screen.getByRole("button", { name: "Subscribe" }));
    await waitFor(() => expect(app.ui.dialog).toBeNull());
    const added = app.doc.shares.find((s) => s.name === "Imported share");
    expect(app.doc.nodes.find((n) => n.id === added?.rootNodeId)?.parentId).toBeNull();
  });
});

describe("ShareFolderDialog", () => {
  it("share_folder_creates_identity_when_missing", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ save: "/tmp/lab.portolano-share" }));
    expect(await ipc.identityGet()).toBeNull();
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    app.ui.open({ type: "shareFolder", folderId: lab.id });
    await screen.findByRole("dialog", { name: "Share “Lab”" });

    // The identity name is required first.
    await fireEvent.click(await screen.findByRole("button", { name: "Browse…" }));
    await waitFor(() => expect((screen.getByLabelText("Share file") as HTMLInputElement).value).toBe("/tmp/lab.portolano-share"));
    await fireEvent.click(screen.getByRole("button", { name: "Publish" }));
    expect(await screen.findByText("Enter your name.")).toBeTruthy();
    expect(app.doc.shares).toHaveLength(0);

    await fireEvent.input(screen.getByLabelText("Your name"), { target: { value: "Ada Rossi" } });
    await fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    // Published, then the one-time backup step.
    await screen.findByRole("dialog", { name: "Back up your identity" });
    expect((await ipc.identityGet())?.name).toBe("Ada Rossi");
    expect(app.doc.shares.filter((s) => s.role === "owner")).toHaveLength(1);
  });

  it("share_folder_backup_asks_the_password_twice_and_wipes_it", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ save: "/tmp/x.portolano-share" }));
    const exportSpy = vi.spyOn(ipc, "identityExport");
    await ipc.identityCreate("Ada");
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    app.ui.open({ type: "shareFolder", folderId: lab.id });
    await screen.findByRole("dialog", { name: "Share “Lab”" });
    await fireEvent.click(await screen.findByRole("button", { name: "Browse…" }));
    await waitFor(() => expect((screen.getByLabelText("Share file") as HTMLInputElement).value).not.toBe(""));
    await fireEvent.click(screen.getByRole("button", { name: "Publish" }));
    await screen.findByRole("dialog", { name: "Back up your identity" });

    await fireEvent.input(screen.getByLabelText("Backup password"), { target: { value: "one" } });
    await fireEvent.input(screen.getByLabelText("Repeat the password"), { target: { value: "two" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save backup…" }));
    expect(await screen.findByText("The two passwords are different.")).toBeTruthy();
    expect(exportSpy).not.toHaveBeenCalled();

    await fireEvent.input(screen.getByLabelText("Repeat the password"), { target: { value: "one" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save backup…" }));
    await waitFor(() => expect(app.ui.dialog).toBeNull());
    expect(exportSpy).toHaveBeenCalledTimes(1);
    expect(exportSpy).toHaveBeenCalledWith("/tmp/x.portolano-share", "one");
  });

  it("share_folder_title_caps_a_huge_folder_name", async () => {
    const { app } = await start();
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    await app.doc.upsert({ ...lab, name: "x".repeat(5000) });
    app.ui.open({ type: "shareFolder", folderId: lab.id });
    const dialog = await screen.findByRole("dialog", { name: /^Share “x+…”$/ });
    expect(dialog.getAttribute("aria-label") ?? dialog.textContent!.length).toBeTruthy();
    expect(screen.getAllByText(/^Share “x+…”$/)[0]!.textContent!.length).toBeLessThan(120);
  });

  it("share_folder_with_a_locked_vault_never_offers_create", async () => {
    const { app, ipc } = await start({ vault: "locked" });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await ipc.identityCreate("Ada Rossi");
    await lockVault(app);
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    app.ui.open({ type: "shareFolder", folderId: lab.id });
    await cancelUnlock();
    expect(await screen.findByText("Unlock the vault to see your identity.")).toBeTruthy();
    expect(screen.queryByLabelText("Your name")).toBeNull();
    expect((screen.getByRole("button", { name: "Publish" }) as HTMLButtonElement).disabled).toBe(true);

    await fireEvent.click(
      within(screen.getByRole("dialog", { name: "Share “Lab”" })).getByRole("button", { name: "Unlock…" }),
    );
    await unlockWithPassword();
    await waitFor(() => expect(screen.queryByText("Unlock the vault to see your identity.")).toBeNull());
    expect(screen.queryByLabelText("Your name")).toBeNull();
  });
});

describe("document shares", () => {
  it("unsubscribing_from_a_share_that_is_already_gone_refreshes_the_tree", async () => {
    const { app, ipc } = await start({ shares: true });
    expect(app.doc.shares.some((s) => s.id === MOCK_SHARE_IDS.link)).toBe(true);
    // The backend drops the share behind the UI's back.
    await ipc.shareUnsubscribe(MOCK_SHARE_IDS.link);
    const result = await app.doc.unsubscribe(MOCK_SHARE_IDS.link);
    expect(result.ok).toBe(false);
    expect(app.doc.shares.some((s) => s.id === MOCK_SHARE_IDS.link)).toBe(false);
  });
});

describe("ShareInfoDialog", () => {
  it("share_info_confirm_clears_pending", async () => {
    const { app } = await start({ shares: true });
    expect(app.doc.shares[0]?.pendingTargets).toBe(1);
    app.ui.open({ type: "shareInfo", linkId: MOCK_SHARE_IDS.link });
    await screen.findByRole("dialog", { name: "Shared folder “Team lab”" });
    expect(screen.getByText("Ada Rossi")).toBeTruthy();
    expect(screen.getByText("7F3A-91C2-0D4E-B6A8")).toBeTruthy();
    expect(screen.getByText(/1 connection\(s\)/)).toBeTruthy();
    const hostRow = screen.getByText("Gateway: ssh gw.team.example");
    const confirm = screen.getByRole("button", { name: "Confirm" });
    expect(hostRow.compareDocumentPosition(confirm) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    await fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Confirm" })).toBeNull());
    expect(app.doc.shares[0]?.pendingTargets).toBe(0);
  });

  it("share_info_confirm_conflict_reloads_without_an_error", async () => {
    const { app, ipc } = await start({ shares: true });
    app.ui.open({ type: "shareInfo", linkId: MOCK_SHARE_IDS.link });
    await screen.findByRole("dialog", { name: "Shared folder “Team lab”" });
    // The share moves on behind the dialog's back: the mock bumps its revision.
    ipc.simulate.bumpShareRevision(MOCK_SHARE_IDS.link);
    await fireEvent.click(screen.getByRole("button", { name: "Confirm" }));
    expect(await screen.findByText(/changed while you were looking at it/)).toBeTruthy();
    expect(screen.queryByText(/Could not confirm/)).toBeNull();
    expect(app.doc.shares[0]?.pendingTargets).toBe(1);
  });

  it("share_info_shows_publish_for_the_owner", async () => {
    const { app, ipc } = await start({ shares: true });
    await ipc.identityCreate("Ada");
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    const link = await ipc.sharePublish(lab.id, "/tmp/lab.portolano-share");
    await app.doc.reload();
    app.ui.open({ type: "shareInfo", linkId: link.id });
    await screen.findByRole("dialog", { name: "Shared folder “Lab”" });
    expect(screen.getByText("/tmp/lab.portolano-share")).toBeTruthy();
    await fireEvent.click(screen.getByRole("button", { name: "Publish update" }));
    await waitFor(() => expect(app.doc.shares.find((s) => s.id === link.id)?.revision).toBe(2));
  });
});

describe("ShareInfoDialog with a locked vault", () => {
  it("publish_update_asks_to_unlock_and_stops_quietly_on_cancel", async () => {
    const { app, ipc } = await start({ shares: true, vault: "locked" });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await ipc.identityCreate("Ada");
    const lab = app.doc.nodes.find((n) => n.type === "folder" && n.name === "Lab")!;
    const link = await ipc.sharePublish(lab.id, "/tmp/lab.portolano-share");
    await app.doc.reload();
    await lockVault(app);
    const republish = vi.spyOn(ipc, "shareRepublish");
    app.ui.open({ type: "shareInfo", linkId: link.id });
    await screen.findByRole("dialog", { name: "Shared folder “Lab”" });
    await fireEvent.click(screen.getByRole("button", { name: "Publish update" }));
    await cancelUnlock();
    expect(republish).not.toHaveBeenCalled();
    expect(screen.queryByText(/Could not publish/)).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("IdentitySection", () => {
  async function openSettings(ipc: MockIpc, app: AppState): Promise<void> {
    app.ui.open({ type: "settings" });
    await screen.findByRole("dialog", { name: "Settings" });
    await screen.findByRole("heading", { name: "Identity" });
    void ipc;
  }

  it("identity_section_hides_import_when_identity_exists", async () => {
    const { app, ipc } = await start();
    await openSettings(ipc, app);
    // No identity: create and import are offered.
    expect(await screen.findByRole("button", { name: "Import…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Create identity" })).toBeTruthy();

    await fireEvent.input(screen.getByLabelText("Your name"), { target: { value: "Ada Rossi" } });
    await fireEvent.click(screen.getByRole("button", { name: "Create identity" }));
    expect(await screen.findByRole("button", { name: "Export backup…" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Copy public code" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Import…" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Create identity" })).toBeNull();
  });

  it("identity_section_shows_an_existing_identity_at_once", async () => {
    const { app, ipc } = await start();
    await ipc.identityCreate("Ada Rossi");
    await openSettings(ipc, app);
    expect(await screen.findByText("A1B2-C3D4-E5F6-0718")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Import…" })).toBeNull();
  });

  it("identity_section_imports_a_backup_and_wipes_the_password", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ open: "/tmp/id.portolano-id" }));
    await openSettings(ipc, app);
    await fireEvent.click(await screen.findByRole("button", { name: "Import…" }));
    const password = (await screen.findByLabelText("Password of the backup")) as HTMLInputElement;
    await fireEvent.input(password, { target: { value: "secret" } });
    await fireEvent.click(screen.getByRole("button", { name: "Import identity" }));
    expect(await screen.findByRole("button", { name: "Export backup…" })).toBeTruthy();
    expect(screen.queryByLabelText("Password of the backup")).toBeNull();
  });
});

describe("IdentitySection with a locked vault", () => {
  it("identity_section_locked_shows_unlock_and_never_create_or_import", async () => {
    const { app, ipc } = await start({ vault: "locked" });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await ipc.identityCreate("Ada Rossi");
    await lockVault(app);
    app.ui.open({ type: "settings" });
    await screen.findByRole("dialog", { name: "Settings" });
    // Opening Settings never prompts for the master password.
    expect(await screen.findByText("Unlock the vault to see your signing identity")).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Create identity" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Import…" })).toBeNull();
    expect(screen.queryByLabelText("Your name")).toBeNull();

    await fireEvent.click(
      within(screen.getByRole("dialog", { name: "Settings" })).getByRole("button", { name: "Unlock…" }),
    );
    await unlockWithPassword();
    expect(await screen.findByText("A1B2-C3D4-E5F6-0718")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Create identity" })).toBeNull();
  });

  it("identity_section_reacts_to_the_vault_being_unlocked_elsewhere", async () => {
    const { app, ipc } = await start({ vault: "locked" });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await ipc.identityCreate("Ada Rossi");
    await lockVault(app);
    app.ui.open({ type: "settings" });
    await screen.findByText("Unlock the vault to see your signing identity");
    // While the identity is being read, neither Create nor Import may flash.
    const real = ipc.identityGet.bind(ipc);
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => (release = resolve));
    const pending = vi.spyOn(ipc, "identityGet").mockImplementation(async () => {
      await gate;
      return real();
    });
    await app.vault.unlock(MOCK_MASTER_PASSWORD);
    await waitFor(() => expect(pending).toHaveBeenCalled());
    const settings = screen.getByRole("dialog", { name: "Settings" });
    expect(within(settings).queryByRole("button", { name: "Create identity" })).toBeNull();
    expect(within(settings).queryByRole("button", { name: "Import…" })).toBeNull();
    expect(within(settings).queryByText("Unlock the vault to see your signing identity")).toBeNull();
    release();
    expect(await screen.findByText("A1B2-C3D4-E5F6-0718")).toBeTruthy();
    expect(screen.queryByText("Unlock the vault to see your signing identity")).toBeNull();
    await lockVault(app);
    expect(await screen.findByText("Unlock the vault to see your signing identity")).toBeTruthy();
    expect(screen.queryByText("A1B2-C3D4-E5F6-0718")).toBeNull();
  });
});

describe("IdentitySection status errors", () => {
  it("identity_section_shows_a_retry_when_the_vault_status_cannot_be_read", async () => {
    const { app, ipc } = await start({ vault: "locked" });
    app.vault.status = "unknown";
    vi.spyOn(ipc, "vaultStatus").mockRejectedValueOnce({ code: "internal", message: "boom" });
    app.ui.open({ type: "settings" });
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    const retry = await within(settings).findByRole("button", { name: "Retry" });
    expect(within(settings).getByText(/boom/)).toBeTruthy();
    await fireEvent.click(retry);
    expect(await within(settings).findByText("Unlock the vault to see your signing identity")).toBeTruthy();
    expect(within(settings).queryByText(/boom/)).toBeNull();
  });

  it("identity_section_clears_its_error_when_the_vault_locks", async () => {
    const { app, ipc } = await start();
    vi.spyOn(ipc, "identityGet").mockRejectedValueOnce({ code: "internal", message: "bad read" });
    app.ui.open({ type: "settings" });
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    expect(await within(settings).findByText(/bad read/)).toBeTruthy();
    await lockVault(app);
    await within(settings).findByText("Unlock the vault to see your signing identity");
    expect(within(settings).queryByText(/bad read/)).toBeNull();
  });
});

describe("IdentitySection without a vault", () => {
  it("identity_section_without_a_vault_offers_create_vault_and_never_prompts", async () => {
    const { app } = await start({ vault: "notCreated" });
    app.ui.open({ type: "settings" });
    await screen.findByRole("dialog", { name: "Settings" });
    const settings = screen.getByRole("dialog", { name: "Settings" });
    expect(await within(settings).findByRole("button", { name: "Create vault…" })).toBeTruthy();
    expect(screen.queryByRole("dialog", { name: "Create the vault" })).toBeNull();
    expect(screen.queryByRole("dialog", { name: "Unlock the vault" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Create identity" })).toBeNull();
  });
});

describe("password wiping", () => {
  it("backup_export_passes_the_password_then_the_reopened_form_is_empty", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ save: "/tmp/id.portolano-id" }));
    await ipc.identityCreate("Ada");
    const exportSpy = vi.spyOn(ipc, "identityExport");
    app.ui.open({ type: "settings" });
    await fireEvent.click(await screen.findByRole("button", { name: "Export backup…" }));
    await fireEvent.input(await screen.findByLabelText("Backup password"), { target: { value: "pw1" } });
    await fireEvent.input(screen.getByLabelText("Repeat the password"), { target: { value: "pw1" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save backup…" }));
    await waitFor(() => expect(exportSpy).toHaveBeenCalledWith("/tmp/id.portolano-id", "pw1"));
    await waitFor(() => expect(screen.queryByLabelText("Backup password")).toBeNull());

    await fireEvent.click(screen.getByRole("button", { name: "Export backup…" }));
    expect((await screen.findByLabelText("Backup password") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Repeat the password") as HTMLInputElement).value).toBe("");
  });

  it("backup_export_failure_still_wipes_the_fields", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ save: "/tmp/id.portolano-id" }));
    await ipc.identityCreate("Ada");
    vi.spyOn(ipc, "identityExport").mockRejectedValueOnce(new Error("disk full"));
    app.ui.open({ type: "settings" });
    await fireEvent.click(await screen.findByRole("button", { name: "Export backup…" }));
    await fireEvent.input(await screen.findByLabelText("Backup password"), { target: { value: "pw1" } });
    await fireEvent.input(screen.getByLabelText("Repeat the password"), { target: { value: "pw1" } });
    await fireEvent.click(screen.getByRole("button", { name: "Save backup…" }));
    await waitFor(() =>
      expect((screen.getByLabelText("Backup password") as HTMLInputElement).value).toBe(""),
    );
    expect((screen.getByLabelText("Repeat the password") as HTMLInputElement).value).toBe("");
  });

  it("import_passes_the_password_and_a_failed_import_wipes_the_field", async () => {
    const { app, ipc } = await start({}, createMockFilePicker({ open: "/tmp/id.portolano-id" }));
    const importSpy = vi.spyOn(ipc, "identityImport").mockRejectedValueOnce(new Error("wrong password"));
    app.ui.open({ type: "settings" });
    await fireEvent.click(await screen.findByRole("button", { name: "Import…" }));
    await fireEvent.input(await screen.findByLabelText("Password of the backup"), { target: { value: "bad" } });
    await fireEvent.click(screen.getByRole("button", { name: "Import identity" }));
    await waitFor(() => expect(importSpy).toHaveBeenCalledWith("/tmp/id.portolano-id", "bad"));
    await waitFor(() =>
      expect((screen.getByLabelText("Password of the backup") as HTMLInputElement).value).toBe(""),
    );
  });
});

describe("menu", () => {
  it("add_remote_folder_menu_opens_the_dialog", async () => {
    const { app, ipc } = await start();
    ipc.simulate.addRemoteFolderMenu();
    await screen.findByRole("dialog", { name: "Add a remote folder" });
    expect(app.ui.dialog?.type).toBe("addRemoteFolder");
  });
});
