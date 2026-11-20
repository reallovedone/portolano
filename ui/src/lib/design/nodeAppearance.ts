// Which icons and colours a connection or folder may be given. Plain names only (no components),
// so the mock IPC can validate with them like Rust does. The lists, and their order, are those of
// `crates/core-model/src/appearance.json`; nodeAppearance.test.ts fails if they ever differ.
// The icons themselves are in icons/nodeIcons.ts; each colour is a `--node-<name>` token.

export const NODE_ICON_NAMES = [
  "server",
  "database",
  "monitor",
  "terminal",
  "globe",
  "cloud",
  "folder",
  "building",
  "house",
  "shield",
  "router",
  "network",
  "hard-drive",
  "cpu",
  "box",
  "container",
  "lock",
  "key",
  "star",
  "flag",
  "briefcase",
  "wrench",
  "flask-conical",
  "laptop",
  "smartphone",
  "printer",
  "camera",
  "radio",
  "zap",
  "activity",
  "code",
  "bug",
  "cable",
  "wifi",
  "mail",
  "users",
  "layers",
  "package",
  "rocket",
  "heart",
] as const;

export const NODE_COLORS = ["red", "orange", "yellow", "green", "teal", "blue", "purple", "pink", "gray"] as const;

export type NodeIconName = (typeof NODE_ICON_NAMES)[number];
export type NodeColor = (typeof NODE_COLORS)[number];

export function isNodeIcon(value: unknown): value is NodeIconName {
  return typeof value === "string" && (NODE_ICON_NAMES as readonly string[]).includes(value);
}

export function isNodeColor(value: unknown): value is NodeColor {
  return typeof value === "string" && (NODE_COLORS as readonly string[]).includes(value);
}
