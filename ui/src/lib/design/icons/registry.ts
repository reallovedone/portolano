// The interface icon set: Lucide (ISC licence, bundled by @lucide/svelte, nothing is fetched at
// run time). Only the icons listed here end up in the bundle, thanks to per-icon imports.
// Add an entry here before using a new icon; keep the set small and consistent.

import type { Component } from "svelte";

import ArrowLeft from "@lucide/svelte/icons/arrow-left";
import ArrowRight from "@lucide/svelte/icons/arrow-right";
import Check from "@lucide/svelte/icons/check";
import ChevronDown from "@lucide/svelte/icons/chevron-down";
import ChevronRight from "@lucide/svelte/icons/chevron-right";
import ChevronUp from "@lucide/svelte/icons/chevron-up";
import ChevronsDownUp from "@lucide/svelte/icons/chevrons-down-up";
import ChevronsUpDown from "@lucide/svelte/icons/chevrons-up-down";
import CircleAlert from "@lucide/svelte/icons/circle-exclamation-point";
import CircleCheck from "@lucide/svelte/icons/circle-check";
import CircleX from "@lucide/svelte/icons/circle-x";
import Clipboard from "@lucide/svelte/icons/clipboard";
import Clock from "@lucide/svelte/icons/clock";
import Copy from "@lucide/svelte/icons/copy";
import Download from "@lucide/svelte/icons/download";
import Ellipsis from "@lucide/svelte/icons/ellipsis";
import ExternalLink from "@lucide/svelte/icons/external-link";
import Eye from "@lucide/svelte/icons/eye";
import EyeOff from "@lucide/svelte/icons/eye-off";
import Folder from "@lucide/svelte/icons/folder";
import FolderOpen from "@lucide/svelte/icons/folder-open";
import FolderPlus from "@lucide/svelte/icons/folder-plus";
import Globe from "@lucide/svelte/icons/globe";
import House from "@lucide/svelte/icons/house";
import Info from "@lucide/svelte/icons/info";
import Key from "@lucide/svelte/icons/key-round";
import Keyboard from "@lucide/svelte/icons/keyboard";
import Link from "@lucide/svelte/icons/link";
import Lock from "@lucide/svelte/icons/lock";
import LockOpen from "@lucide/svelte/icons/lock-open";
import Minus from "@lucide/svelte/icons/minus";
import Monitor from "@lucide/svelte/icons/monitor";
import PanelLeft from "@lucide/svelte/icons/panel-left";
import Pencil from "@lucide/svelte/icons/pencil";
import Plug from "@lucide/svelte/icons/plug";
import Plus from "@lucide/svelte/icons/plus";
import Power from "@lucide/svelte/icons/power";
import RefreshCw from "@lucide/svelte/icons/refresh-cw";
import RotateCw from "@lucide/svelte/icons/rotate-cw";
import Search from "@lucide/svelte/icons/search";
import Server from "@lucide/svelte/icons/server";
import Settings from "@lucide/svelte/icons/settings";
import Shield from "@lucide/svelte/icons/shield";
import ShieldAlert from "@lucide/svelte/icons/shield-exclamation-point";
import ShieldCheck from "@lucide/svelte/icons/shield-check";
import Tag from "@lucide/svelte/icons/tag";
import Terminal from "@lucide/svelte/icons/terminal";
import Trash from "@lucide/svelte/icons/trash";
import TriangleAlert from "@lucide/svelte/icons/triangle-exclamation-point";
import Unplug from "@lucide/svelte/icons/unplug";
import Upload from "@lucide/svelte/icons/upload";
import WifiOff from "@lucide/svelte/icons/wifi-off";
import X from "@lucide/svelte/icons/x";

type IconComponent = Component<any>;

export const ICONS = {
  "arrow-left": ArrowLeft,
  "arrow-right": ArrowRight,
  check: Check,
  "chevron-down": ChevronDown,
  "chevron-right": ChevronRight,
  "chevron-up": ChevronUp,
  "chevrons-down-up": ChevronsDownUp,
  "chevrons-up-down": ChevronsUpDown,
  clipboard: Clipboard,
  clock: Clock,
  close: X,
  copy: Copy,
  download: Download,
  edit: Pencil,
  error: CircleAlert,
  "external-link": ExternalLink,
  eye: Eye,
  "eye-off": EyeOff,
  folder: Folder,
  "folder-open": FolderOpen,
  "folder-plus": FolderPlus,
  globe: Globe,
  house: House,
  info: Info,
  key: Key,
  keyboard: Keyboard,
  link: Link,
  lock: Lock,
  "lock-open": LockOpen,
  minus: Minus,
  monitor: Monitor,
  more: Ellipsis,
  "panel-left": PanelLeft,
  plug: Plug,
  plus: Plus,
  power: Power,
  refresh: RefreshCw,
  "rotate-cw": RotateCw,
  search: Search,
  server: Server,
  settings: Settings,
  shield: Shield,
  "shield-alert": ShieldAlert,
  "shield-check": ShieldCheck,
  success: CircleCheck,
  tag: Tag,
  terminal: Terminal,
  trash: Trash,
  unplug: Unplug,
  upload: Upload,
  warning: TriangleAlert,
  "wifi-off": WifiOff,
  "x-circle": CircleX,
} satisfies Record<string, IconComponent>;

export type IconName = keyof typeof ICONS;

export const ICON_NAMES = Object.keys(ICONS) as IconName[];
