// Shared folders of the mock IPC: one mounted share (with a connection whose target waits for
// the user's confirmation) and the commands around it. Nothing is read from or written to disk
// and nothing is signed: a path containing "broken" is refused like a damaged share file.

import { IpcError } from "./errors";
import type {
  CredentialRef,
  Document,
  DocumentView,
  IdentitySummary,
  Node,
  RemoteNodeInfo,
  ShareLocation,
  SharePreview,
  ShareSummary,
  Uuid,
} from "./types";

const id = (n: number): Uuid => `00000000-0000-4000-8000-${n.toString(16).padStart(12, "0")}`;

/** Fixed ids of the demo share, so tests can refer to it. */
export const MOCK_SHARE_IDS = {
  link: id(201),
  root: id(202),
  servers: id(203),
  gateway: id(204),
  metrics: id(205),
} as const;

interface Link {
  summary: ShareSummary;
  /** Local folder the share is mounted under (null: the top level). */
  parentId: Uuid | null;
  /** The mounted nodes, root first (subscriber links only). */
  nodes: Node[];
}

export interface MockShares {
  /** The document with the mounted shares, plus what the UI needs to tell them apart. */
  view(doc: Document): DocumentView;
  /** The merged node list (what the sessions of the mock see). */
  nodes(doc: Document): Node[];
  isRemote(nodeId: Uuid): boolean;
  isRoot(nodeId: Uuid): boolean;
  /** Links mounted under a local folder in `folders`. */
  linksUnder(folders: ReadonlySet<Uuid>): Uuid[];
  /** Moves a mount root to another local folder. */
  setParent(rootId: Uuid, parentId: Uuid | null): void;
  /** The share moves to the next revision without telling the UI (a sync it has not seen yet). */
  bumpRevision(linkId: Uuid): void;
  /** Tells the subscribed UI that the shares changed (a sync would do this). */
  notifyChanged(): void;
  onChanged(handler: () => void): () => void;
  readonly api: {
    identityGet(): Promise<IdentitySummary | null>;
    identityCreate(name: string): Promise<IdentitySummary>;
    identityExport(path: string, password: string): Promise<void>;
    identityImport(path: string, password: string): Promise<IdentitySummary>;
    sharePublish(folderId: Uuid, path: string, doc: Document): Promise<ShareSummary>;
    shareRepublish(linkId: Uuid): Promise<ShareSummary>;
    sharePreview(location: ShareLocation): Promise<SharePreview>;
    shareSubscribe(previewId: Uuid, parentId: Uuid | null, doc: Document): Promise<ShareSummary>;
    shareSync(linkId: Uuid | null): Promise<ShareSummary[]>;
    shareUnsubscribe(linkId: Uuid): Promise<void>;
    shareSetCredential(nodeId: Uuid, credential: CredentialRef): Promise<void>;
    shareConfirmTargets(linkId: Uuid, revision: number): Promise<void>;
  };
}

function seedLink(): Link {
  const folder = (nodeId: Uuid, parentId: Uuid | null, name: string): Node => ({
    type: "folder",
    id: nodeId,
    parentId,
    name,
    credential: { type: "inherit" },
  });
  return {
    parentId: null,
    summary: {
      id: MOCK_SHARE_IDS.link,
      name: "Team lab",
      role: "subscriber",
      rootNodeId: MOCK_SHARE_IDS.root,
      ownerName: "Ada Rossi",
      fingerprint: "7F3A-91C2-0D4E-B6A8",
      revision: 3,
      signerName: "Ada Rossi",
      publishedAt: "2026-11-02T09:30:00Z",
      lastSync: "2026-11-20T08:00:00Z",
      lastError: null,
      pendingTargets: 1,
      pendingHosts: [
        {
          nodeId: MOCK_SHARE_IDS.gateway,
          name: "Gateway",
          protocol: "ssh",
          host: "gw.team.example",
          port: null,
          username: null,
        },
      ],
      location: { type: "url", value: "https://shares.example/team-lab.portolano" },
      publishPath: null,
    },
    nodes: [
      folder(MOCK_SHARE_IDS.root, null, "Team lab"),
      folder(MOCK_SHARE_IDS.servers, MOCK_SHARE_IDS.root, "Servers"),
      {
        type: "connection",
        id: MOCK_SHARE_IDS.gateway,
        parentId: MOCK_SHARE_IDS.servers,
        name: "Gateway",
        host: "gw.team.example",
        port: null,
        credential: { type: "inherit" },
        username: null,
        tags: ["shared"],
        notes: "",
        options: { type: "ssh", jumpHost: null, keepaliveSecs: null },
      },
      {
        type: "connection",
        id: MOCK_SHARE_IDS.metrics,
        parentId: MOCK_SHARE_IDS.root,
        name: "Metrics",
        host: "metrics.team.example",
        port: null,
        credential: { type: "inherit" },
        username: null,
        tags: [],
        notes: "",
        options: { type: "web", url: "https://metrics.team.example/", isolatedProfile: false },
      },
    ],
  };
}

export function createMockShares(seed: boolean): MockShares {
  let links: Link[] = seed ? [seedLink()] : [];
  const overrides = new Map<Uuid, CredentialRef>();
  const previews = new Map<Uuid, SharePreview & { location: ShareLocation }>();
  const handlers = new Set<() => void>();
  let identity: IdentitySummary | null = null;

  const clone = <T>(value: T): T => structuredClone(value);
  const linkOf = (linkId: Uuid): Link => {
    const link = links.find((l) => l.summary.id === linkId);
    if (!link) throw new IpcError("not_found", "This shared folder is not available.");
    return link;
  };
  const remoteLink = (nodeId: Uuid): Link | undefined =>
    links.find((l) => l.nodes.some((n) => n.id === nodeId));
  const nowIso = (): string => new Date().toISOString();
  const requireIdentity = (): IdentitySummary => {
    if (!identity) throw new IpcError("not_found", "There is no signing identity yet.");
    return identity;
  };
  const makeIdentity = (name: string): IdentitySummary => ({
    id: crypto.randomUUID(),
    name,
    fingerprint: "A1B2-C3D4-E5F6-0718",
    publicCode: `portolano-id:1:mock-${encodeURIComponent(name)}`,
  });

  const mountedNodes = (): Node[] =>
    links.flatMap((l) =>
      l.nodes.map((n) => {
        const root = n.id === l.summary.rootNodeId;
        const override = overrides.get(n.id);
        const withParent = root ? { ...n, parentId: l.parentId } : n;
        return clone(override ? { ...withParent, credential: override } : withParent);
      }),
    );

  const api: MockShares["api"] = {
    async identityGet() {
      return identity ? clone(identity) : null;
    },
    async identityCreate(name) {
      if (identity) throw new IpcError("conflict", "This vault already has a signing identity.");
      const clean = name.trim();
      if (clean === "" || clean.length > 100) {
        throw new IpcError("invalid_input", "The name must have between 1 and 100 characters.");
      }
      identity = makeIdentity(clean);
      return clone(identity);
    },
    async identityExport(path, password) {
      requireIdentity();
      if (path.trim() === "" || password === "") {
        throw new IpcError("invalid_input", "A file and a password are needed.");
      }
    },
    async identityImport(path, password) {
      if (identity) throw new IpcError("conflict", "This vault already has a signing identity.");
      if (path.includes("broken")) {
        throw new IpcError("invalid_input", "This is not an identity backup file.");
      }
      if (password === "") throw new IpcError("auth_failed", "Wrong password.");
      identity = makeIdentity("Imported identity");
      return clone(identity);
    },
    async sharePublish(folderId, path, doc) {
      const owner = requireIdentity();
      const folder = doc.nodes.find((n) => n.id === folderId);
      if (!folder || folder.type !== "folder") throw new IpcError("not_found", "Folder not found.");
      if (links.some((l) => l.summary.role === "owner" && l.summary.rootNodeId === folderId)) {
        throw new IpcError("conflict", "This folder is already shared.");
      }
      const link: Link = {
        parentId: null,
        nodes: [],
        summary: {
          id: crypto.randomUUID(),
          name: folder.name,
          role: "owner",
          rootNodeId: folderId,
          ownerName: owner.name,
          fingerprint: owner.fingerprint,
          revision: 1,
          signerName: owner.name,
          publishedAt: nowIso(),
          lastSync: null,
          lastError: null,
          pendingTargets: 0,
          pendingHosts: [],
          location: null,
          publishPath: path,
        },
      };
      links = [...links, link];
      return clone(link.summary);
    },
    async shareRepublish(linkId) {
      const link = linkOf(linkId);
      if (link.summary.role !== "owner") {
        throw new IpcError("invalid_input", "Only the owner can publish an update.");
      }
      link.summary = { ...link.summary, revision: link.summary.revision + 1, publishedAt: nowIso() };
      return clone(link.summary);
    },
    async sharePreview(location) {
      if (location.value.includes("broken")) {
        throw new IpcError("invalid_input", "This is not a valid share file.");
      }
      if (location.type === "url" && !location.value.startsWith("https://")) {
        throw new IpcError("invalid_input", "Only https addresses are accepted.");
      }
      const preview: SharePreview & { location: ShareLocation } = {
        previewId: crypto.randomUUID(),
        name: "Imported share",
        ownerName: "Ada Rossi",
        fingerprint: "7F3A-91C2-0D4E-B6A8",
        revision: 1,
        folders: 1,
        connections: 2,
        skipped: [],
        hosts: [
          { nodeId: crypto.randomUUID(), name: "Gateway", protocol: "ssh", host: "gw.team.example", port: null, username: "ops" },
          { nodeId: crypto.randomUUID(), name: "Desk", protocol: "rdp", host: "10.0.0.5", port: 3390, username: null },
        ],
        location,
      };
      previews.set(preview.previewId, preview);
      const { location: _location, ...shown } = preview;
      return shown;
    },
    async shareSubscribe(previewId, parentId, doc) {
      const preview = previews.get(previewId);
      if (!preview) throw new IpcError("not_found", "This preview is no longer available.");
      if (parentId !== null) {
        const parent = doc.nodes.find((n) => n.id === parentId);
        if (!parent && remoteLink(parentId)) {
          throw new IpcError("read_only", "This item belongs to a shared folder and cannot be changed here.");
        }
        if (!parent || parent.type !== "folder") {
          throw new IpcError("invalid_input", "The parent is not a folder.");
        }
      }
      previews.delete(previewId);
      const rootId = crypto.randomUUID();
      const linkId = crypto.randomUUID();
      const link: Link = {
        parentId,
        summary: {
          id: linkId,
          name: preview.name,
          role: "subscriber",
          rootNodeId: rootId,
          ownerName: preview.ownerName,
          fingerprint: preview.fingerprint,
          revision: preview.revision,
          signerName: preview.ownerName,
          publishedAt: nowIso(),
          lastSync: nowIso(),
          lastError: null,
          pendingTargets: 0,
          pendingHosts: [],
          location: preview.location,
          publishPath: null,
        },
        nodes: [
          { type: "folder", id: rootId, parentId: null, name: preview.name, credential: { type: "inherit" } },
        ],
      };
      links = [...links, link];
      return clone(link.summary);
    },
    async shareSync(linkId) {
      const targets = linkId === null ? links : [linkOf(linkId)];
      for (const link of targets) {
        if (link.summary.role === "subscriber") link.summary = { ...link.summary, lastSync: nowIso() };
      }
      return targets.map((l) => clone(l.summary));
    },
    async shareUnsubscribe(linkId) {
      const link = linkOf(linkId);
      links = links.filter((l) => l !== link);
      for (const node of link.nodes) overrides.delete(node.id);
    },
    async shareSetCredential(nodeId, credential) {
      if (!remoteLink(nodeId)) {
        throw new IpcError("invalid_input", "This item does not belong to a shared folder.");
      }
      if (credential.type === "inherit") overrides.delete(nodeId);
      else overrides.set(nodeId, clone(credential));
    },
    async shareConfirmTargets(linkId, revision) {
      const link = linkOf(linkId);
      if (link.summary.revision !== revision) {
        throw new IpcError(
          "conflict",
          "The shared folder changed since you looked at it: review it again.",
        );
      }
      link.summary = { ...link.summary, pendingTargets: 0, pendingHosts: [] };
    },
  };

  return {
    view(doc) {
      const remoteNodes: Record<Uuid, RemoteNodeInfo> = {};
      for (const link of links) {
        for (const node of link.nodes) {
          remoteNodes[node.id] = { linkId: link.summary.id, isRoot: node.id === link.summary.rootNodeId };
        }
      }
      return {
        ...clone(doc),
        nodes: [...clone(doc.nodes), ...mountedNodes()],
        shares: links.map((l) => clone(l.summary)),
        remoteNodes,
      };
    },
    nodes: (doc) => [...doc.nodes, ...mountedNodes()],
    isRemote: (nodeId) => remoteLink(nodeId) !== undefined,
    isRoot: (nodeId) => links.some((l) => l.summary.rootNodeId === nodeId && l.nodes.length > 0),
    linksUnder: (folders) =>
      links
        .filter((l) => l.summary.role === "subscriber" && l.parentId !== null && folders.has(l.parentId))
        .map((l) => l.summary.id),
    setParent(rootId, parentId) {
      const link = links.find((l) => l.summary.rootNodeId === rootId && l.nodes.length > 0);
      if (link) link.parentId = parentId;
    },
    bumpRevision(linkId) {
      const link = linkOf(linkId);
      link.summary = { ...link.summary, revision: link.summary.revision + 1 };
    },
    notifyChanged() {
      for (const handler of [...handlers]) handler();
    },
    onChanged(handler) {
      handlers.add(handler);
      return () => handlers.delete(handler);
    },
    api,
  };
}
