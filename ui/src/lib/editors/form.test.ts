import { describe, expect, it } from "vitest";

import type { Connection } from "../ipc";
import {
  draftFromConnection,
  draftToConnection,
  draftToFolder,
  newConnectionDraft,
  newFolderDraft,
  parseTags,
  validateConnectionDraft,
  validateFolderDraft,
  type ConnectionDraft,
} from "./form";

const newId = (): string => "generated-id";

function valid(kind: ConnectionDraft["kind"], patch: Partial<ConnectionDraft> = {}): ConnectionDraft {
  return {
    ...newConnectionDraft(kind, null),
    name: "n",
    host: "h.example",
    url: "https://example.org/",
    ...patch,
  };
}

describe("validateConnectionDraft", () => {
  it("requires a name for every kind", () => {
    for (const kind of ["localShell", "ssh", "rdp", "vnc", "web"] as const) {
      expect(validateConnectionDraft(valid(kind, { name: "  " })).name).toBe("validation.nameRequired");
    }
  });

  it("requires a host only for ssh, rdp and vnc", () => {
    for (const kind of ["ssh", "rdp", "vnc"] as const) {
      expect(validateConnectionDraft(valid(kind, { host: " " })).host).toBe("validation.hostRequired");
    }
    expect(validateConnectionDraft(valid("localShell", { host: "" }))).toEqual({});
    expect(validateConnectionDraft(valid("web", { host: "" }))).toEqual({});
  });

  it("accepts empty ports and 1..65535, rejects the rest", () => {
    expect(validateConnectionDraft(valid("ssh", { port: "" })).port).toBeUndefined();
    expect(validateConnectionDraft(valid("ssh", { port: "22" })).port).toBeUndefined();
    expect(validateConnectionDraft(valid("ssh", { port: "65535" })).port).toBeUndefined();
    for (const bad of ["0", "65536", "-1", "2x", "1.5", "abc"]) {
      expect(validateConnectionDraft(valid("rdp", { port: bad })).port).toBe("validation.portRange");
    }
  });

  it("validates ssh keepalive", () => {
    expect(validateConnectionDraft(valid("ssh", { keepaliveSecs: "30" }))).toEqual({});
    expect(validateConnectionDraft(valid("ssh", { keepaliveSecs: "0" })).keepaliveSecs).toBe(
      "validation.keepalive",
    );
    expect(validateConnectionDraft(valid("ssh", { keepaliveSecs: "x" })).keepaliveSecs).toBe(
      "validation.keepalive",
    );
  });

  it("requires rdp width and height together", () => {
    expect(validateConnectionDraft(valid("rdp", { width: "1280", height: "800" }))).toEqual({});
    expect(validateConnectionDraft(valid("rdp", { width: "1280" })).height).toBe(
      "validation.dimensionPair",
    );
    expect(validateConnectionDraft(valid("rdp", { height: "800" })).width).toBe(
      "validation.dimensionPair",
    );
    expect(validateConnectionDraft(valid("rdp", { width: "0", height: "800" })).width).toBe(
      "validation.dimension",
    );
    expect(validateConnectionDraft(valid("rdp", { width: "99999", height: "800" })).width).toBe(
      "validation.dimension",
    );
  });

  it("requires a valid http(s) URL for web", () => {
    expect(validateConnectionDraft(valid("web", { url: "" })).url).toBe("validation.urlRequired");
    expect(validateConnectionDraft(valid("web", { url: "not a url" })).url).toBe(
      "validation.urlInvalid",
    );
    expect(validateConnectionDraft(valid("web", { url: "ftp://x" })).url).toBe("validation.urlInvalid");
    expect(validateConnectionDraft(valid("web", { url: "http://localhost:8080/x" }))).toEqual({});
  });

  it("requires a credential when 'specific' is chosen", () => {
    const draft = valid("ssh", { credential: { mode: "id", id: "" } });
    expect(validateConnectionDraft(draft).credential).toBe("validation.credentialRequired");
    draft.credential = { mode: "id", id: "c1" };
    expect(validateConnectionDraft(draft).credential).toBeUndefined();
  });
});

describe("draftToConnection", () => {
  it("builds an ssh connection with defaults as null", () => {
    const c = draftToConnection(valid("ssh", { name: " box ", host: " h ", tags: "a, b ,A,,c" }), newId);
    expect(c).toMatchObject({
      id: "generated-id",
      name: "box",
      host: "h",
      port: null,
      tags: ["a", "b", "c"],
      credential: { type: "inherit" },
      options: { type: "ssh", jumpHost: null, keepaliveSecs: null },
    });
  });

  it("derives host from the URL for web and clears it for local shells", () => {
    const web = draftToConnection(valid("web", { url: " https://Intranet.example:8443/a " }), newId);
    expect(web.host).toBe("intranet.example");
    expect(web.port).toBeNull();
    expect(web.options).toEqual({
      type: "web",
      url: "https://Intranet.example:8443/a",
      isolatedProfile: false,
    });
    const shell = draftToConnection(valid("localShell", { host: "ignored", args: "-l\n\n -x " }), newId);
    expect(shell.host).toBe("");
    expect(shell.options).toEqual({ type: "localShell", program: null, args: ["-l", "-x"], cwd: null });
  });

  it("keeps the id when editing and maps credentials", () => {
    const draft = valid("vnc", { id: "keep", credential: { mode: "id", id: "c9" }, port: "5901" });
    const c = draftToConnection(draft, newId);
    expect(c.id).toBe("keep");
    expect(c.port).toBe(5901);
    expect(c.credential).toEqual({ type: "id", id: "c9" });
    expect(draftToConnection(valid("vnc", { credential: { mode: "none", id: "" } }), newId).credential).toEqual({
      type: "none",
    });
  });

  it("round-trips every kind through a draft", () => {
    const base = { id: "i", parentId: "p", name: "n", port: null, notes: "memo", tags: ["t"] };
    // Only protocols that authenticate keep a user name.
    const samples: Connection[] = [
      { ...base, host: "", username: null, credential: { type: "inherit" }, options: { type: "localShell", program: "/bin/zsh", args: ["-l"], cwd: "/tmp" } },
      { ...base, host: "h", port: 2222, username: "deploy", credential: { type: "id", id: "c" }, options: { type: "ssh", jumpHost: "j", keepaliveSecs: 15 } },
      { ...base, host: "h", port: 3390, username: "CORP\\ann", credential: { type: "none" }, options: { type: "rdp", width: 800, height: 600, domain: "D", ignoreCert: true, allowWithoutNla: true, kerberosKdc: "tcp://dc.example.com:88", shareClipboard: false, macShortcuts: true } },
      { ...base, host: "h", username: null, credential: { type: "inherit" }, options: { type: "vnc", viewOnly: true } },
      { ...base, host: "example.org", username: null, credential: { type: "inherit" }, options: { type: "web", url: "https://example.org/", isolatedProfile: true } },
    ];
    for (const sample of samples) {
      const draft = draftFromConnection(sample);
      expect(validateConnectionDraft(draft)).toEqual({});
      expect(draftToConnection(draft, newId)).toEqual(sample);
    }
  });
});

describe("RDP clipboard sharing", () => {
  const base = { id: "i", parentId: null, name: "n", port: null, notes: "", tags: [] as string[] };
  const rdp = (shareClipboard: boolean): Connection => ({
    ...base,
    host: "h",
    username: null,
    credential: { type: "inherit" },
    options: {
      type: "rdp",
      width: null,
      height: null,
      domain: null,
      ignoreCert: false,
      allowWithoutNla: false,
      kerberosKdc: null,
      shareClipboard,
      macShortcuts: false,
    },
  });

  it("is off in a new RDP draft and in a document that predates the option", () => {
    expect(newConnectionDraft("rdp", null).shareClipboard).toBe(false);
    const legacy = rdp(true);
    // A document saved before the option existed has no such key.
    delete (legacy.options as { shareClipboard?: boolean }).shareClipboard;
    expect(draftFromConnection(legacy).shareClipboard).toBe(false);
  });

  it("is kept through a draft, on and off", () => {
    for (const on of [true, false]) {
      const converted = draftToConnection(draftFromConnection(rdp(on)), newId);
      expect(converted.options).toMatchObject({ type: "rdp", shareClipboard: on });
    }
  });
});

describe("RDP Kerberos KDC", () => {
  const kdcError = (value: string) =>
    validateConnectionDraft(valid("rdp", { kerberosKdc: value })).kerberosKdc;

  it("is empty in a new draft, a blank value is saved as none", () => {
    expect(newConnectionDraft("rdp", null).kerberosKdc).toBe("");
    expect(draftToConnection(valid("rdp"), newId).options).toMatchObject({ kerberosKdc: null });
    expect(draftToConnection(valid("rdp", { kerberosKdc: "   " }), newId).options).toMatchObject({
      kerberosKdc: null,
    });
  });

  it("is trimmed and saved", () => {
    const saved = draftToConnection(valid("rdp", { kerberosKdc: "  dc.example.com:88 " }), newId);
    expect(saved.options).toMatchObject({ type: "rdp", kerberosKdc: "dc.example.com:88" });
  });

  it("loads empty for a document that predates the field", () => {
    const c = draftToConnection(valid("rdp", { kerberosKdc: "dc.example.com" }), newId);
    delete (c.options as { kerberosKdc?: string | null }).kerberosKdc;
    expect(draftFromConnection(c).kerberosKdc).toBe("");
  });

  it("accepts every documented form and an empty value", () => {
    for (const ok of [
      "",
      "dc.example.com",
      "dc.example.com:88",
      "10.0.0.5",
      "[fe80::1]",
      "tcp://dc.example.com",
      "tcp://dc.example.com:88",
      "udp://10.0.0.5:88",
      "https://proxy.example.com/KdcProxy",
    ]) {
      expect(kdcError(ok), ok).toBeUndefined();
    }
  });

  it("refuses an invalid value with a message instead of saving it", () => {
    for (const bad of ["dc example.com", "tcp://", "dc.example.com:notaport", "dc.example.com:70000", "tcp://dc.example.com/path"]) {
      expect(kdcError(bad), bad).toBe("validation.kdcInvalid");
    }
    expect(kdcError("http://proxy.example.com/KdcProxy")).toBe("validation.kdcScheme");
    expect(kdcError("ldap://dc.example.com")).toBe("validation.kdcScheme");
    expect(kdcError("tcp://ann:pw@dc.example.com")).toBe("validation.kdcInvalid");
  });

  it("is only checked for RDP", () => {
    expect(validateConnectionDraft(valid("ssh", { kerberosKdc: "bad value" })).kerberosKdc).toBeUndefined();
  });
});

describe("parseTags", () => {
  it("trims, drops empties and de-duplicates ignoring case", () => {
    expect(parseTags(" a, ,B,b ,a")).toEqual(["a", "B"]);
    expect(parseTags("")).toEqual([]);
  });
});

describe("folder drafts", () => {
  it("validates and converts", () => {
    const draft = newFolderDraft("p");
    expect(validateFolderDraft(draft).name).toBe("validation.nameRequired");
    draft.name = " Servers ";
    draft.credential = { mode: "id", id: "" };
    expect(validateFolderDraft(draft).credential).toBe("validation.credentialRequired");
    draft.credential = { mode: "id", id: "c1" };
    expect(validateFolderDraft(draft)).toEqual({});
    expect(draftToFolder(draft, newId)).toEqual({
      id: "generated-id",
      parentId: "p",
      name: "Servers",
      credential: { type: "id", id: "c1" },
    });
  });

  it("keeps the optional user name for SSH, RDP and VNC and drops it elsewhere", () => {
    const withName = (kind: ConnectionDraft["kind"]) =>
      draftToConnection(valid(kind, { username: "  ann  " }), newId).username;
    expect(withName("ssh")).toBe("ann");
    expect(withName("rdp")).toBe("ann");
    expect(withName("vnc")).toBe("ann");
    expect(withName("web")).toBeNull();
    expect(withName("localShell")).toBeNull();
    // Empty or blank means "no user name".
    expect(draftToConnection(valid("ssh", { username: "   " }), newId).username).toBeNull();
    expect(draftToConnection(valid("ssh"), newId).username).toBeNull();
  });

  it("a connection saved before the field existed loads with an empty user name", () => {
    const old = { ...draftToConnection(valid("ssh"), newId) } as Partial<Connection>;
    delete old.username;
    const draft = draftFromConnection(old as Connection);
    expect(draft.username).toBe("");
    expect(draftToConnection(draft, newId).username).toBeNull();
  });
});

describe("RDP Mac shortcuts", () => {
  const base = { id: "i", parentId: null, name: "n", port: null, notes: "", tags: [] as string[] };
  const rdp = (macShortcuts: boolean): Connection => ({
    ...base,
    host: "h",
    username: null,
    credential: { type: "inherit" },
    options: {
      type: "rdp",
      width: null,
      height: null,
      domain: null,
      ignoreCert: false,
      allowWithoutNla: false,
      kerberosKdc: null,
      shareClipboard: false,
      macShortcuts,
    },
  });

  it("is on in a new RDP draft on macOS only", () => {
    expect(newConnectionDraft("rdp", null, "macos").macShortcuts).toBe(true);
    expect(newConnectionDraft("rdp", null, "windows").macShortcuts).toBe(false);
    expect(newConnectionDraft("rdp", null, "other").macShortcuts).toBe(false);
  });

  it("is off for a document that predates the option, even when opened on macOS", () => {
    const legacy = rdp(true);
    delete (legacy.options as { macShortcuts?: boolean }).macShortcuts;
    expect(draftFromConnection(legacy).macShortcuts).toBe(false);
  });

  it("is kept through a draft, on and off", () => {
    for (const on of [true, false]) {
      const converted = draftToConnection(draftFromConnection(rdp(on)), newId);
      expect(converted.options).toMatchObject({ type: "rdp", macShortcuts: on });
    }
  });
});
