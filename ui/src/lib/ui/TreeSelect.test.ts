// TreeSelect: a pop-up button whose list shows a hierarchy with real indentation.
import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import TreeSelect from "./TreeSelect.svelte";
import { openTreeSelect, pickTreeValue, treeSelectOptions } from "./treeSelectTesting";

const OPTIONS = [
  { value: "", label: "(top level)", icon: null },
  { value: "co", label: "Company", depth: 0 },
  { value: "rdp", label: "Servers", depth: 1 },
  { value: "de", label: "Germany", depth: 2 },
  { value: "it", label: "Italy", depth: 2, disabled: true },
  { value: "ssh", label: "Shells", depth: 1 },
];

afterEach(cleanup);

function mount(value = "de", onchange = vi.fn()) {
  render(TreeSelect, { props: { label: "Location", options: OPTIONS, value, onchange } });
  return { control: screen.getByLabelText("Location"), onchange };
}

describe("TreeSelect", () => {
  it("is a labelled combobox that shows the selection with its ancestors", () => {
    const { control } = mount("de");
    expect(control.getAttribute("role")).toBe("combobox");
    expect(control.getAttribute("aria-expanded")).toBe("false");
    expect(control.dataset.value).toBe("de");
    expect(control.querySelector(".current")?.textContent).toBe("Germany");
    expect([...control.querySelectorAll(".ancestor")].map((n) => n.textContent)).toEqual(["Company", "Servers"]);
  });

  it("lists every option with its depth and marks the selected and the disabled ones", async () => {
    const { control } = mount("de");
    const list = await openTreeSelect(control);
    expect(list.getAttribute("role")).toBe("listbox");
    expect(list.parentElement).toBe(document.body);
    const selected = list.querySelector('[aria-selected="true"]');
    expect(selected?.querySelector(".name")?.textContent).toBe("Germany");
    await fireEvent.click(control);
    const options = await treeSelectOptions(control);
    expect(options.map((o) => o.depth)).toEqual([0, 0, 1, 2, 2, 1]);
    expect(options.filter((o) => o.disabled).map((o) => o.label)).toEqual(["Italy"]);
  });

  it("chooses with the pointer, reports the change and closes", async () => {
    const { control, onchange } = mount("de");
    await pickTreeValue(control, "ssh");
    expect(onchange).toHaveBeenCalledWith("ssh");
    expect(control.dataset.value).toBe("ssh");
    expect(control.getAttribute("aria-expanded")).toBe("false");
    expect(document.querySelector('[role="listbox"]')).toBeNull();
  });

  it("does not choose a disabled option", async () => {
    const { control, onchange } = mount("de");
    await pickTreeValue(control, "it");
    expect(onchange).not.toHaveBeenCalled();
    expect(control.dataset.value).toBe("de");
  });

  it("works from the keyboard and skips disabled options", async () => {
    const { control, onchange } = mount("de");
    await fireEvent.keyDown(control, { key: "ArrowDown" });
    expect(control.getAttribute("aria-expanded")).toBe("true");
    const activeLabel = (): string | null | undefined =>
      document.getElementById(control.getAttribute("aria-activedescendant") ?? "")?.querySelector(".name")
        ?.textContent;
    expect(activeLabel()).toBe("Germany");
    await fireEvent.keyDown(control, { key: "ArrowDown" });
    expect(activeLabel()).toBe("Shells"); // "Italy" is disabled
    await fireEvent.keyDown(control, { key: "Home" });
    expect(activeLabel()).toBe("(top level)");
    await fireEvent.keyDown(control, { key: "End" });
    expect(activeLabel()).toBe("Shells");
    await fireEvent.keyDown(control, { key: "Enter" });
    expect(onchange).toHaveBeenCalledWith("ssh");
    expect(control.getAttribute("aria-expanded")).toBe("false");
  });

  it("closes on Escape without letting the key reach a surrounding dialog", async () => {
    const outer = vi.fn();
    document.body.addEventListener("keydown", outer);
    const { control, onchange } = mount("de");
    await fireEvent.click(control);
    await fireEvent.keyDown(control, { key: "Escape" });
    expect(control.getAttribute("aria-expanded")).toBe("false");
    expect(outer).not.toHaveBeenCalled();
    expect(onchange).not.toHaveBeenCalled();
    // With the list closed, Escape is no longer swallowed.
    await fireEvent.keyDown(control, { key: "Escape" });
    expect(outer).toHaveBeenCalledTimes(1);
    document.body.removeEventListener("keydown", outer);
  });

  it("jumps to the next matching option while typing", async () => {
    const { control } = mount("");
    await fireEvent.click(control);
    await fireEvent.keyDown(control, { key: "s" });
    const active = document.getElementById(control.getAttribute("aria-activedescendant") ?? "");
    expect(active?.querySelector(".name")?.textContent).toBe("Servers");
  });

  it("closes when the pointer goes down elsewhere", async () => {
    const { control } = mount("de");
    await fireEvent.click(control);
    await fireEvent.pointerDown(document.body);
    expect(control.getAttribute("aria-expanded")).toBe("false");
  });
});
