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
      { ...base, host: "h", port: 3390, username: "CORP\\ann", credential: { type: "none" }, options: { type: "rdp", width: 800, height: 600, domain: "D", ignoreCert: true, allowWithoutNla: true } },
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
