// Choosing files with the system's native dialogs. A small injectable service: the real one
// uses `tauri-plugin-dialog`, the mock one (plain browser, tests) returns canned paths. The UI
// only ever receives a path; the backend reads or writes the file.

export interface FileFilter {
  name: string;
  /** Extensions without the dot. */
  extensions: string[];
}

export interface OpenFileOptions {
  title: string;
  /** Omit to show every file (private keys usually have no extension). */
  filters?: FileFilter[];
  /** Start in the user's `~/.ssh` folder when it can be found; otherwise the OS default. */
  startInSshDir?: boolean;
}

export interface SaveFileOptions {
  title: string;
  /** Suggested file name (no folder). */
  defaultName: string;
  filters?: FileFilter[];
}

export interface FilePicker {
  /** The chosen path, or `null` when the user cancelled. */
  pickOpen(options: OpenFileOptions): Promise<string | null>;
  pickSave(options: SaveFileOptions): Promise<string | null>;
}

/**
 * The native dialogs (`tauri-plugin-dialog`, permissions `dialog:allow-open` and
 * `dialog:allow-save`). Loaded lazily: the plugin API does not exist in a plain browser.
 */
/** `~/.ssh`, or `undefined` when the home folder cannot be resolved (the dialog then uses its default). */
async function sshDirectory(): Promise<string | undefined> {
  try {
    const { homeDir, join } = await import("@tauri-apps/api/path");
    return await join(await homeDir(), ".ssh");
  } catch {
    // No default folder is a normal fallback, not an error worth showing.
    return undefined;
  }
}

export const tauriFilePicker: FilePicker = {
  async pickOpen({ title, filters, startInSshDir }) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const defaultPath = startInSshDir ? await sshDirectory() : undefined;
    const chosen = await open({ title, filters, defaultPath, multiple: false, directory: false });
    return typeof chosen === "string" ? chosen : null;
  },
  async pickSave({ title, defaultName, filters }) {
    const { save } = await import("@tauri-apps/plugin-dialog");
    return (await save({ title, defaultPath: defaultName, filters })) ?? null;
  },
};

export interface MockFilePickerOptions {
  /** Path "chosen" by the open dialog; `null` simulates cancelling. */
  open?: string | null;
  /** Path "chosen" by the save dialog; `null` simulates cancelling. */
  save?: string | null;
}

/** What the mock picker was asked, for tests. */
export interface MockFilePicker extends FilePicker {
  readonly openRequests: OpenFileOptions[];
  readonly saveRequests: SaveFileOptions[];
}

/**
 * Canned dialogs. By default the open dialog "chooses" a MobaXterm export and the save dialog
 * "chooses" the suggested name in a demo folder.
 */
export function createMockFilePicker(options: MockFilePickerOptions = {}): MockFilePicker {
  const openRequests: OpenFileOptions[] = [];
  const saveRequests: SaveFileOptions[] = [];
  return {
    openRequests,
    saveRequests,
    async pickOpen(request) {
      openRequests.push(request);
      return options.open === undefined ? "/Users/demo/Sessions.mxtsessions" : options.open;
    },
    async pickSave(request) {
      saveRequests.push(request);
      return options.save === undefined ? `/Users/demo/${request.defaultName}` : options.save;
    },
  };
}
