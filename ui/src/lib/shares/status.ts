// The state of a shared folder as one icon, tone and label: used next to the folder in the tree
// and in "Share info…", so both always say the same thing.

import { t, type StringKey } from "../i18n/strings";
import type { ShareSummary } from "../ipc";
import type { IconName } from "../design/icons/registry";
import { ERROR_MAX, displayText } from "./display";

export type ShareIndicatorKind = "ok" | "owned" | "offline" | "rejected" | "error" | "folderDeleted" | "pending";

export interface ShareIndicator {
  kind: ShareIndicatorKind;
  icon: IconName;
  /** Colour of the icon: quiet when nothing is wrong, louder the more it needs the user. */
  tone: "quiet" | "muted" | "warning" | "danger";
  label: string;
}

const ICONS: Record<ShareIndicatorKind, { icon: IconName; tone: ShareIndicator["tone"]; key: StringKey }> = {
  ok: { icon: "link", tone: "quiet", key: "share.status.ok" },
  owned: { icon: "link", tone: "quiet", key: "share.status.owned" },
  offline: { icon: "wifi-off", tone: "muted", key: "share.status.offline" },
  rejected: { icon: "warning", tone: "warning", key: "share.status.rejected" },
  error: { icon: "warning", tone: "danger", key: "share.status.error" },
  folderDeleted: { icon: "warning", tone: "danger", key: "share.status.folderDeleted" },
  pending: { icon: "shield-alert", tone: "warning", key: "share.status.pending" },
};

/** What to show for a share. A failure wins over hosts waiting for confirmation. */
export function shareIndicator(share: Pick<ShareSummary, "role" | "status" | "pendingTargets">): ShareIndicator {
  let kind: ShareIndicatorKind;
  if (share.status !== "ok") kind = share.status;
  else if (share.pendingTargets > 0) kind = "pending";
  else kind = share.role === "owner" ? "owned" : "ok";
  const entry = ICONS[kind];
  return { kind, icon: entry.icon, tone: entry.tone, label: t(entry.key) };
}

/** Tooltip text: the state, why, and when the folder was last brought up to date. */
export function shareTooltip(
  share: Pick<ShareSummary, "role" | "status" | "pendingTargets" | "lastError" | "lastSync">,
): string {
  const indicator = shareIndicator(share);
  const lines = [indicator.label];
  if (indicator.kind === "pending") {
    const key = share.pendingTargets === 1 ? "share.status.pendingDetail.one" : "share.status.pendingDetail.other";
    lines.push(t(key, { count: share.pendingTargets }));
  }
  else if (share.lastError && indicator.kind !== "folderDeleted") lines.push(displayText(share.lastError, ERROR_MAX));
  if (share.role === "subscriber") {
    const when = share.lastSync ? new Date(share.lastSync).toLocaleString() : t("share.infoNever");
    lines.push(t("share.status.lastSync", { when }));
  }
  return lines.join("\n");
}
