import { toIpcError, type Ipc, type IpcError, type Node, type Uuid } from "../ipc";
import { isStringArray, loadJson, saveJson } from "./storage";

export type Result<T> = { ok: true; value: T } | { ok: false; error: IpcError };

const EXPANDED_KEY = "portolano.tree.expanded";

async function attempt<T>(run: () => Promise<T>): Promise<Result<T>> {
  try {
    return { ok: true, value: await run() };
  } catch (e) {
    return { ok: false, error: toIpcError(e) };
  }
}

/** The connection document (flat node list) plus the tree expansion state. */
export class DocumentStore {
  nodes = $state<Node[]>([]);
  status = $state<"loading" | "ready" | "error">("loading");
  loadError = $state<IpcError | null>(null);
  expanded = $state<Set<Uuid>>(new Set(loadJson(EXPANDED_KEY, [], isStringArray)));
  readonly #ipc: Ipc;

  constructor(ipc: Ipc) {
    this.#ipc = ipc;
  }

  node(id: Uuid): Node | undefined {
    return this.nodes.find((n) => n.id === id);
  }

  async load(): Promise<boolean> {
    this.status = "loading";
    return this.#refresh();
  }

  /** Re-reads the document without the loading state: the backend changed it on its own. */
  async reload(): Promise<boolean> {
    return this.#refresh();
  }

  async #refresh(): Promise<boolean> {
    const result = await attempt(() => this.#ipc.docGet());
    if (result.ok) {
      this.nodes = result.value.nodes;
      this.loadError = null;
      this.status = "ready";
      return true;
    }
    this.loadError = result.error;
    this.status = "error";
    return false;
  }

  /** Persists a node and refreshes the local copy from the backend's answer. */
  async upsert(node: Node): Promise<Result<Node>> {
    // Snapshot: IPC payloads must be plain data, not reactive proxies.
    const result = await attempt(() => this.#ipc.docUpsertNode($state.snapshot(node) as Node));
    if (result.ok) {
      const stored = result.value;
      const exists = this.nodes.some((n) => n.id === stored.id);
      this.nodes = exists
        ? this.nodes.map((n) => (n.id === stored.id ? stored : n))
        : [...this.nodes, stored];
    }
    return result;
  }

  /** Deletes a node and its subtree (the backend does the cascade; we reload to stay exact). */
  async remove(id: Uuid): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.docDeleteNode(id));
    if (result.ok) await this.#refresh();
    return result;
  }

  async move(id: Uuid, newParentId: Uuid | null): Promise<Result<void>> {
    const result = await attempt(() => this.#ipc.docMoveNode(id, newParentId));
    if (result.ok) {
      this.nodes = this.nodes.map((n) => (n.id === id ? { ...n, parentId: newParentId } : n));
      if (newParentId !== null) this.expand(newParentId);
    }
    return result;
  }

  isExpanded(id: Uuid): boolean {
    return this.expanded.has(id);
  }

  setExpanded(id: Uuid, value: boolean): void {
    const next = new Set(this.expanded);
    if (value) next.add(id);
    else next.delete(id);
    this.expanded = next;
    saveJson(EXPANDED_KEY, [...next]);
  }

  expand(id: Uuid): void {
    if (!this.expanded.has(id)) this.setExpanded(id, true);
  }

  collapseAll(): void {
    this.expanded = new Set();
    saveJson(EXPANDED_KEY, []);
  }
}
