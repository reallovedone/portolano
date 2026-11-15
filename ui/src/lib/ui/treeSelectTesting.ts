// Test helpers for TreeSelect: the list is rendered at the end of <body> while it is open.
import { fireEvent } from "@testing-library/svelte";

/** Opens the pop-up of a TreeSelect button and returns its listbox. */
export async function openTreeSelect(control: HTMLElement): Promise<HTMLElement> {
  if (control.getAttribute("aria-expanded") !== "true") await fireEvent.click(control);
  const list = document.getElementById(control.getAttribute("aria-controls") ?? "");
  if (!list) throw new Error("TreeSelect list did not open");
  return list;
}

/** Chooses the option with the given value (for folders: the folder id, "" for the top level). */
export async function pickTreeValue(control: HTMLElement, value: string): Promise<void> {
  const list = await openTreeSelect(control);
  const option = [...list.querySelectorAll<HTMLElement>('[role="option"]')].find(
    (candidate) => candidate.dataset.value === value,
  );
  if (!option) throw new Error(`TreeSelect has no option with value "${value}"`);
  await fireEvent.click(option);
}

export interface TreeOptionInfo {
  label: string;
  depth: number;
  disabled: boolean;
}

/** Lists the options of a TreeSelect, then closes the pop-up again. */
export async function treeSelectOptions(control: HTMLElement): Promise<TreeOptionInfo[]> {
  const list = await openTreeSelect(control);
  const options = [...list.querySelectorAll<HTMLElement>('[role="option"]')].map((option) => ({
    label: option.querySelector(".name")?.textContent?.trim() ?? "",
    depth: Number(option.style.getPropertyValue("--depth") || 0),
    disabled: option.getAttribute("aria-disabled") === "true",
  }));
  await fireEvent.click(control);
  return options;
}
