// Import and export of the mock IPC. Nothing is read from or written to disk: `importPreview`
// answers with canned content that depends on the format, and a few path names trigger the
// error codes of the real backend so the error states can be exercised by hand:
//   a path containing "encrypted" -> unsupported, "missing" -> io, "broken" -> invalid_input.

import { IpcError } from "./errors";
import type {
  Document,
  ExportFormat,
  ImportFormat,
  ImportPreview,
  ImportWarning,
  Node,
  Uuid,
} from "./types";

export interface MockExport {
  format: ExportFormat;
  path: string;
  nodeIds: Uuid[] | null;
}

export interface MockTransfer {
  preview(format: ImportFormat, path: string): Promise<ImportPreview>;
  apply(previewId: Uuid, targetFolderId: Uuid | null): Promise<number>;
  export(format: ExportFormat, path: string, nodeIds: Uuid[] | null): Promise<void>;
  /** Every successful export so far (tests). */
  readonly exports: readonly MockExport[];
}

/** The sentence the real importers use for every ignored password. */
const PASSWORD_IGNORED = "password ignored: passwords are never imported";

function connection(
  parentId: Uuid | null,
  name: string,
  host: string,
  options: Extract<Node, { type: "connection" }>["options"],
  port: number | null = null,
): Node {
  return {
    type: "connection",
    id: crypto.randomUUID(),
    parentId,
    name,
    host,
    port,
    credential: { type: "none" },
    username: null,
    tags: [],
    notes: "",
    options,
  };
}

function folder(parentId: Uuid | null, name: string): Node {
  return { type: "folder", id: crypto.randomUUID(), parentId, name, credential: { type: "inherit" } };
}

const ssh = { type: "ssh", jumpHost: null, keepaliveSecs: null } as const;
const rdp = { type: "rdp", width: null, height: null, domain: null, ignoreCert: false, allowWithoutNla: false, shareClipboard: false, macShortcuts: false } as const;

function cannedPreview(format: ImportFormat): { nodes: Node[]; warnings: ImportWarning[] } {
  if (format === "rdp") {
    return {
      nodes: [connection(null, "Reception desk", "reception.example.org", rdp, 3390)],
      warnings: [{ message: PASSWORD_IGNORED, line: 7 }],
    };
  }
  const datacenter = folder(null, "Datacenter");
  const web = folder(datacenter.id, "Web tier");
  const windows = folder(datacenter.id, "Windows");
  const nodes: Node[] = [
    datacenter,
    web,
    windows,
    connection(web.id, "web-01", "web01.example.org", ssh),
    connection(web.id, "web-02", "web02.example.org", ssh),
    connection(windows.id, "File server", "files.example.org", rdp),
    connection(windows.id, "Print server", "print.example.org", rdp),
    connection(windows.id, "Terminal host", "tse.example.org", rdp, 3390),
    connection(null, "Jump box", "jump.example.org", ssh, 2222),
    connection(null, "Lab screen", "lab.example.org", { type: "vnc", viewOnly: false }, 5901),
    connection(null, "Status page", "status.example.org", {
      type: "web",
      url: "https://status.example.org/",
      isolatedProfile: false,
    }),
  ];
  const warnings: ImportWarning[] = [
    { message: PASSWORD_IGNORED, line: 4 },
    { message: PASSWORD_IGNORED, line: 9 },
    { message: PASSWORD_IGNORED, line: 15 },
    { message: "session 'old-switch' skipped: Telnet is not supported", line: 21 },
    { message: "invalid port ignored for 'backup': 70000 is not a valid port", line: 27 },
    { message: "no connections found in the folder 'Archive'", line: null },
  ];
  return { nodes, warnings };
}

export function createMockTransfer(
  getDoc: () => Document,
  setDoc: (doc: Document) => void,
): MockTransfer {
  const previews = new Map<Uuid, Node[]>();
  const exports: MockExport[] = [];

  return {
    exports,
    async preview(format, path) {
      if (path.includes("missing")) throw new IpcError("io", "i/o error: no such file or directory");
      if (path.includes("encrypted")) {
        throw new IpcError("unsupported", "not supported: the file is encrypted (mRemoteNG)");
      }
      if (path.includes("broken")) {
        throw new IpcError("invalid_input", "invalid input: the file is not valid for this format");
      }
      const { nodes, warnings } = cannedPreview(format);
      const previewId = crypto.randomUUID();
      previews.set(previewId, nodes);
      return structuredClone({ previewId, nodes, warnings });
    },
    async apply(previewId, targetFolderId) {
      const nodes = previews.get(previewId);
      if (!nodes) throw new IpcError("not_found", "the import preview was not found or has expired");
      const doc = getDoc();
      if (targetFolderId !== null) {
        const target = doc.nodes.find((n) => n.id === targetFolderId);
        if (!target) throw new IpcError("not_found", `target folder ${targetFolderId} not found`);
        if (target.type !== "folder") throw new IpcError("invalid_input", "the target is not a folder");
      }
      const added = structuredClone(nodes).map((n) => (n.parentId === null ? { ...n, parentId: targetFolderId } : n));
      setDoc({ ...doc, nodes: [...doc.nodes, ...added] });
      previews.delete(previewId);
      return added.length;
    },
    async export(format, path, nodeIds) {
      if (path.trim() === "") throw new IpcError("invalid_input", "the path is empty");
      if (path.includes("readonly")) throw new IpcError("io", "i/o error: permission denied");
      if (format === "rdp") {
        const only = nodeIds?.length === 1 ? getDoc().nodes.find((n) => n.id === nodeIds[0]) : undefined;
        if (only?.type !== "connection" || only.options.type !== "rdp") {
          throw new IpcError("unsupported", "not supported: only one RDP connection can be exported as .rdp");
        }
      }
      exports.push({ format, path, nodeIds: nodeIds === null ? null : [...nodeIds] });
    },
  };
}

/**
 * `readPrivateKeyFile` of the mock: nothing is read from disk. A fake (not a real) key is
 * returned, except for marked paths that trigger the real backend's error codes:
 * "missing" -> not_found, "unreadable" -> io, "big" / "notakey" -> invalid_input.
 */
export async function readMockKeyFile(path: string): Promise<string> {
  if (path.includes("missing")) throw new IpcError("not_found", "the file was not found");
  if (path.includes("unreadable")) throw new IpcError("io", "the file cannot be read: permission denied");
  if (path.includes("big")) throw new IpcError("invalid_input", "the file is too large to be a private key");
  if (path.includes("notakey")) throw new IpcError("invalid_input", "the file does not contain a private key");
  return [
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "bW9jay1rZXktZm9yLXRoZS1kZW1vLW5vdC1hLXJlYWwta2V5",
    "-----END OPENSSH PRIVATE KEY-----",
    "",
  ].join("\n");
}
