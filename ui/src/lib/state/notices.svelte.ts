import { toIpcError } from "../ipc";

export type NoticeKind = "error" | "warning" | "info";

export interface Notice {
  id: number;
  kind: NoticeKind;
  title: string;
  detail: string;
}

/** Visible, dismissible messages (toast area). */
export class NoticeStore {
  items = $state<Notice[]>([]);
  #next = 1;
  #timers = new Map<number, ReturnType<typeof setTimeout>>();

  push(kind: NoticeKind, title: string, detail = ""): number {
    const id = this.#next++;
    this.items = [...this.items, { id, kind, title, detail }];
    // Errors stay until dismissed. A warning is non-blocking and fades more slowly than an
    // information, since it asks the user to do something.
    if (kind === "info") this.#timers.set(id, setTimeout(() => this.dismiss(id), 5000));
    else if (kind === "warning") this.#timers.set(id, setTimeout(() => this.dismiss(id), 12000));
    return id;
  }

  /** Shows an IPC failure with the backend code and message. */
  error(title: string, cause: unknown): void {
    const e = toIpcError(cause);
    this.push("error", title, `${e.message} (${e.code})`);
  }

  dismiss(id: number): void {
    const timer = this.#timers.get(id);
    if (timer !== undefined) {
      clearTimeout(timer);
      this.#timers.delete(id);
    }
    this.items = this.items.filter((n) => n.id !== id);
  }
}
