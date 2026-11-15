import type { Ipc } from "./api";
import { mockIpc } from "./mock";
import { tauriIpc } from "./tauri";

export type IpcMode = "tauri" | "mock";

/**
 * Which implementation to use, in priority order:
 * 1. `?ipc=mock|tauri` in the page URL (run time),
 * 2. `VITE_IPC=mock|tauri` (build time, see `pnpm dev:mock` / `pnpm build:mock`),
 * 3. automatic: `tauri` inside the Tauri webview, `mock` in a plain browser.
 */
export function resolveIpcMode(
  query: string | null,
  env: string | undefined,
  insideTauri: boolean,
): IpcMode {
  for (const choice of [query, env]) {
    if (choice === "mock" || choice === "tauri") return choice;
  }
  return insideTauri ? "tauri" : "mock";
}

function detectMode(): IpcMode {
  const query = new URLSearchParams(globalThis.location?.search ?? "").get("ipc");
  const insideTauri = typeof globalThis === "object" && "__TAURI_INTERNALS__" in globalThis;
  return resolveIpcMode(query, import.meta.env.VITE_IPC, insideTauri);
}

export const ipcMode: IpcMode = detectMode();
export const ipc: Ipc = ipcMode === "mock" ? mockIpc : tauriIpc;

export type { Ipc } from "./api";
export { createMockIpc } from "./mock";
export { IpcError, toIpcError } from "./errors";
export * from "./types";
