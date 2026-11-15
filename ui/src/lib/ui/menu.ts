import type { IconName } from "../design/icons/registry";

export interface MenuItem {
  label: string;
  action: () => void;
  danger?: boolean;
  separatorBefore?: boolean;
  disabled?: boolean;
  /** Shortcut hint shown on the right, e.g. "⌘N". Display only. */
  shortcut?: string;
  /** Optional leading icon. */
  icon?: IconName;
}
