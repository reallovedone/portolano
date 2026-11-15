/// <reference types="svelte" />
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** "mock" or "tauri": forces the IPC implementation at build/run time. */
  readonly VITE_IPC?: string;
}
