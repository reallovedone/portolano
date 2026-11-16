// Form drafts for the connection and folder editors: string-based so inputs can hold
// partially typed values, with validation and conversion to the IPC node types.

import type { Connection, CredentialRef, Folder, Uuid } from "../ipc";
import type { StringKey } from "../i18n/strings";
import type { ConnectionKind } from "../tree/model";

export const DEFAULT_PORTS: Record<ConnectionKind, number | null> = {
  localShell: null,
  ssh: 22,
  rdp: 3389,
  vnc: 5900,
  web: null,
};

/** Kinds whose `host` field is entered by the user. */
export function kindUsesHost(kind: ConnectionKind): boolean {
  return kind === "ssh" || kind === "rdp" || kind === "vnc";
}

export type CredentialMode = CredentialRef["type"];

export interface CredentialDraft {
  mode: CredentialMode;
  /** Only meaningful when `mode` is "id". */
  id: Uuid | "";
}

export interface ConnectionDraft {
  id: Uuid | null;
  parentId: Uuid | null;
  kind: ConnectionKind;
  name: string;
  host: string;
  port: string;
  /** Optional user name that pre-fills the credential prompt (SSH, RDP, VNC). */
  username: string;
  credential: CredentialDraft;
  tags: string;
  notes: string;
  // localShell
  program: string;
  args: string;
  cwd: string;
  // ssh
  jumpHost: Uuid | "";
  keepaliveSecs: string;
  // rdp
  width: string;
  height: string;
  domain: string;
  ignoreCert: boolean;
  allowWithoutNla: boolean;
  shareClipboard: boolean;
  // vnc
  viewOnly: boolean;
  // web
  url: string;
  isolatedProfile: boolean;
}

export type ConnectionField =
  | "name"
  | "host"
  | "port"
  | "credential"
  | "program"
  | "keepaliveSecs"
  | "width"
  | "height"
  | "url";

export type FieldErrors<F extends string> = Partial<Record<F, StringKey>>;

export function newConnectionDraft(
  kind: ConnectionKind,
  parentId: Uuid | null,
): ConnectionDraft {
  return {
    id: null,
    parentId,
    kind,
    name: "",
    host: "",
    port: "",
    username: "",
    credential: { mode: "inherit", id: "" },
    tags: "",
    notes: "",
    program: "",
    args: "",
    cwd: "",
    jumpHost: "",
    keepaliveSecs: "",
    width: "",
    height: "",
    domain: "",
    ignoreCert: false,
    allowWithoutNla: false,
    shareClipboard: false,
    viewOnly: false,
    url: "",
    isolatedProfile: false,
  };
}

export function credentialToDraft(ref: CredentialRef): CredentialDraft {
  return ref.type === "id" ? { mode: "id", id: ref.id } : { mode: ref.type, id: "" };
}

export function draftToCredential(draft: CredentialDraft): CredentialRef {
  if (draft.mode === "id") return { type: "id", id: draft.id };
  return { type: draft.mode };
}

const text = (value: string | null): string => value ?? "";
const num = (value: number | null): string => (value === null ? "" : String(value));

export function draftFromConnection(c: Connection): ConnectionDraft {
  const draft = newConnectionDraft(c.options.type, c.parentId);
  draft.id = c.id;
  draft.name = c.name;
  draft.host = c.host;
  draft.port = num(c.port);
  draft.username = text(c.username);
  draft.credential = credentialToDraft(c.credential);
  draft.tags = c.tags.join(", ");
  draft.notes = c.notes;
  const o = c.options;
  switch (o.type) {
    case "localShell":
      draft.program = text(o.program);
      draft.args = o.args.join("\n");
      draft.cwd = text(o.cwd);
      break;
    case "ssh":
      draft.jumpHost = o.jumpHost ?? "";
      draft.keepaliveSecs = num(o.keepaliveSecs);
      break;
    case "rdp":
      draft.width = num(o.width);
      draft.height = num(o.height);
      draft.domain = text(o.domain);
      draft.ignoreCert = o.ignoreCert;
      draft.allowWithoutNla = o.allowWithoutNla === true;
      draft.shareClipboard = o.shareClipboard === true;
      break;
    case "vnc":
      draft.viewOnly = o.viewOnly;
      break;
    case "web":
      draft.url = o.url;
      draft.isolatedProfile = o.isolatedProfile;
      break;
  }
  return draft;
}

/** Parses a whole number in [min, max]; `null` for an empty string; `undefined` if invalid. */
function parseInteger(value: string, min: number, max: number): number | null | undefined {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  if (!/^\d+$/.test(trimmed)) return undefined;
  const n = Number(trimmed);
  return n >= min && n <= max ? n : undefined;
}

export function parseTags(value: string): string[] {
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const raw of value.split(",")) {
    const tag = raw.trim();
    const key = tag.toLowerCase();
    if (tag !== "" && !seen.has(key)) {
      seen.add(key);
      tags.push(tag);
    }
  }
  return tags;
}

function parseLines(value: string): string[] {
  return value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

function parseWebUrl(value: string): URL | null {
  try {
    const url = new URL(value.trim());
    return url.protocol === "http:" || url.protocol === "https:" ? url : null;
  } catch {
    // An unparsable URL is reported as a validation error by the caller.
    return null;
  }
}

export function validateConnectionDraft(draft: ConnectionDraft): FieldErrors<ConnectionField> {
  const errors: FieldErrors<ConnectionField> = {};
  if (draft.name.trim() === "") errors.name = "validation.nameRequired";

  if (kindUsesHost(draft.kind) && draft.host.trim() === "") errors.host = "validation.hostRequired";
  if (draft.kind !== "localShell" && draft.kind !== "web") {
    if (parseInteger(draft.port, 1, 65535) === undefined) errors.port = "validation.portRange";
  }

  if (draft.credential.mode === "id" && draft.credential.id === "") {
    errors.credential = "validation.credentialRequired";
  }

  switch (draft.kind) {
    case "ssh":
      if (parseInteger(draft.keepaliveSecs, 1, 86400) === undefined) {
        errors.keepaliveSecs = "validation.keepalive";
      }
      break;
    case "rdp": {
      const w = parseInteger(draft.width, 1, 16384);
      const h = parseInteger(draft.height, 1, 16384);
      if (w === undefined) errors.width = "validation.dimension";
      if (h === undefined) errors.height = "validation.dimension";
      if (w !== undefined && h !== undefined && (w === null) !== (h === null)) {
        errors[w === null ? "width" : "height"] = "validation.dimensionPair";
      }
      break;
    }
    case "web":
      if (draft.url.trim() === "") errors.url = "validation.urlRequired";
      else if (parseWebUrl(draft.url) === null) errors.url = "validation.urlInvalid";
      break;
    case "localShell":
    case "vnc":
      break;
  }
  return errors;
}

/** Builds the node to send to `doc_upsert_node`. Call only after validation passed. */
export function draftToConnection(draft: ConnectionDraft, newId: () => Uuid): Connection {
  const orNull = (value: string): string | null => (value.trim() === "" ? null : value.trim());
  let host = draft.host.trim();
  let port: number | null = null;
  if (draft.kind !== "localShell" && draft.kind !== "web") {
    port = parseInteger(draft.port, 1, 65535) ?? null;
  }

  let options: Connection["options"];
  switch (draft.kind) {
    case "localShell":
      host = "";
      options = {
        type: "localShell",
        program: orNull(draft.program),
        args: parseLines(draft.args),
        cwd: orNull(draft.cwd),
      };
      break;
    case "ssh":
      options = {
        type: "ssh",
        jumpHost: draft.jumpHost === "" ? null : draft.jumpHost,
        keepaliveSecs: parseInteger(draft.keepaliveSecs, 1, 86400) ?? null,
      };
      break;
    case "rdp":
      options = {
        type: "rdp",
        width: parseInteger(draft.width, 1, 16384) ?? null,
        height: parseInteger(draft.height, 1, 16384) ?? null,
        domain: orNull(draft.domain),
        ignoreCert: draft.ignoreCert,
        allowWithoutNla: draft.allowWithoutNla,
        shareClipboard: draft.shareClipboard,
      };
      break;
    case "vnc":
      options = { type: "vnc", viewOnly: draft.viewOnly };
      break;
    case "web": {
      const url = parseWebUrl(draft.url);
      host = url?.hostname ?? "";
      options = { type: "web", url: draft.url.trim(), isolatedProfile: draft.isolatedProfile };
      break;
    }
  }

  return {
    id: draft.id ?? newId(),
    parentId: draft.parentId,
    name: draft.name.trim(),
    host,
    port,
    credential: draftToCredential(draft.credential),
    // Only protocols that authenticate have a user name; it is not a secret.
    username: kindUsesHost(draft.kind) ? orNull(draft.username) : null,
    tags: parseTags(draft.tags),
    notes: draft.notes,
    options,
  };
}

// ---- folders ----

export interface FolderDraft {
  id: Uuid | null;
  parentId: Uuid | null;
  name: string;
  credential: CredentialDraft;
}

export function newFolderDraft(parentId: Uuid | null): FolderDraft {
  return { id: null, parentId, name: "", credential: { mode: "inherit", id: "" } };
}

export function draftFromFolder(f: Folder): FolderDraft {
  return {
    id: f.id,
    parentId: f.parentId,
    name: f.name,
    credential: credentialToDraft(f.credential),
  };
}

export function validateFolderDraft(draft: FolderDraft): FieldErrors<"name" | "credential"> {
  const errors: FieldErrors<"name" | "credential"> = {};
  if (draft.name.trim() === "") errors.name = "validation.nameRequired";
  if (draft.credential.mode === "id" && draft.credential.id === "") {
    errors.credential = "validation.credentialRequired";
  }
  return errors;
}

export function draftToFolder(draft: FolderDraft, newId: () => Uuid): Folder {
  return {
    id: draft.id ?? newId(),
    parentId: draft.parentId,
    name: draft.name.trim(),
    credential: draftToCredential(draft.credential),
  };
}
