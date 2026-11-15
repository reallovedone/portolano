// The two entry points of the import and export flows. They talk to the user through the file
// dialogs and the notices; the import preview itself is `ImportDialog`.

import { t } from "../i18n/strings";
import type { ExportFormat, Uuid } from "../ipc";
import type { App } from "../state/app";
import type { FileFilter } from "./files";
import { baseName, safeFileStem } from "./formats";

const IMPORT_FILTERS = (): FileFilter[] => [
  {
    name: t("transfer.filterSupported"),
    extensions: ["mxtsessions", "rdp", "xml", "csv", "json"],
  },
  // An OpenSSH `config` file has no extension, so the dialog must be able to show any file.
  { name: t("transfer.filterAll"), extensions: ["*"] },
];

/** Asks for a file, then opens the import dialog for it. Cancelling the dialog does nothing. */
export async function startImport(app: App): Promise<void> {
  let path: string | null;
  try {
    path = await app.files.pickOpen({ title: t("transfer.openTitle"), filters: IMPORT_FILTERS() });
  } catch (e) {
    app.notices.error(t("transfer.pickFailed"), e);
    return;
  }
  if (path !== null) app.ui.open({ type: "import", path });
}

export interface ExportRequest {
  format: ExportFormat;
  /** `null` exports the whole document. */
  nodeIds: Uuid[] | null;
  /** Name shown in the save dialog without the extension. */
  name: string;
}

/**
 * Asks where to save, then exports. The save dialog and the success message both say that
 * passwords are never part of an export.
 */
export async function startExport(app: App, request: ExportRequest): Promise<void> {
  const json = request.format === "json";
  const extension = json ? "json" : "rdp";
  let path: string | null;
  try {
    path = await app.files.pickSave({
      title: t("transfer.saveTitle"),
      defaultName: `${safeFileStem(request.name)}.${extension}`,
      filters: [
        {
          name: t(json ? "transfer.filterJson" : "transfer.filterRdp"),
          extensions: [extension],
        },
      ],
    });
  } catch (e) {
    app.notices.error(t("transfer.pickFailed"), e);
    return;
  }
  if (path === null) return;
  try {
    await app.ipc.exportDocument(request.format, path, request.nodeIds);
    app.notices.push("info", t("transfer.exported", { file: baseName(path) }));
  } catch (e) {
    app.notices.error(t("transfer.exportFailed"), e);
  }
}
