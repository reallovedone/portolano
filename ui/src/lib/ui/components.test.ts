import { fireEvent, render, screen, within } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { afterEach, describe, expect, it, vi } from "vitest";

import { createMockIpc } from "../ipc";
import { createApp } from "../state/app";
import Badge from "./Badge.svelte";
import Banner from "./Banner.svelte";
import Button from "./Button.svelte";
import Checkbox from "./Checkbox.svelte";
import EmptyState from "./EmptyState.svelte";
import FormRow from "./FormRow.svelte";
import FormSection from "./FormSection.svelte";
import Icon from "./Icon.svelte";
import IconButton from "./IconButton.svelte";
import Kbd from "./Kbd.svelte";
import KindIcon from "./KindIcon.svelte";
import ListRow from "./ListRow.svelte";
import Logo from "./Logo.svelte";
import Radio from "./Radio.svelte";
import RadioGroup from "./RadioGroup.svelte";
import SegmentedControl from "./SegmentedControl.svelte";
import Select from "./Select.svelte";
import Separator from "./Separator.svelte";
import Spinner from "./Spinner.svelte";
import Switch from "./Switch.svelte";
import TextArea from "./TextArea.svelte";
import TextField from "./TextField.svelte";
import TitlebarDragRegion from "./TitlebarDragRegion.svelte";
import Toolbar from "./Toolbar.svelte";
import Tooltip from "./Tooltip.svelte";
import UiHarness from "./UiHarness.svelte";
import UiHarnessHero from "./UiHarnessHero.svelte";
import UiHarnessConfirm from "./UiHarnessConfirm.svelte";
import UiHarnessPrompt from "./UiHarnessPrompt.svelte";
import type { MenuItem } from "./menu";

const text = (value: string) => createRawSnippet(() => ({ render: () => `<span>${value}</span>` }));
const button = (value: string) =>
  createRawSnippet(() => ({ render: () => `<button type="button">${value}</button>` }));

afterEach(() => vi.useRealTimers());

describe("Button", () => {
  it("renders a type=button button with the variant and size classes", () => {
    render(Button, { children: text("Save"), variant: "primary", size: "large" });
    const b = screen.getByRole("button", { name: "Save" });
    expect(b.getAttribute("type")).toBe("button");
    expect(b.classList.contains("primary")).toBe(true);
    expect(b.classList.contains("large")).toBe(true);
  });

  it("calls onclick, and not when disabled", async () => {
    const onclick = vi.fn();
    const { rerender } = render(Button, { children: text("Go"), onclick });
    await fireEvent.click(screen.getByRole("button", { name: "Go" }));
    expect(onclick).toHaveBeenCalledTimes(1);
    await rerender({ children: text("Go"), onclick, disabled: true });
    expect((screen.getByRole("button", { name: "Go" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("loading: busy and aria-disabled, stays focusable, ignores clicks and submits", async () => {
    const onclick = vi.fn();
    render(Button, { children: text("Save"), loading: true, onclick });
    const b = screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;
    expect(b.getAttribute("aria-busy")).toBe("true");
    expect(b.getAttribute("aria-disabled")).toBe("true");
    expect(b.disabled).toBe(false);
    b.focus();
    expect(document.activeElement).toBe(b);
    await fireEvent.click(b);
    expect(onclick).not.toHaveBeenCalled();
  });

  it("passes native attributes (form, data-autofocus, submit type)", () => {
    render(Button, { children: text("Send"), type: "submit", form: "f1", "data-autofocus": "" });
    const b = screen.getByRole("button", { name: "Send" });
    expect(b.getAttribute("type")).toBe("submit");
    expect(b.getAttribute("form")).toBe("f1");
    expect(b.hasAttribute("data-autofocus")).toBe(true);
  });
});

describe("IconButton and Tooltip", () => {
  it("uses the label as accessible name and exposes toggle state", () => {
    render(IconButton, { icon: "plus", label: "New connection", pressed: true });
    const b = screen.getByRole("button", { name: "New connection" });
    expect(b.getAttribute("aria-pressed")).toBe("true");
    expect(b.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("shows its tooltip on keyboard focus and hides it on Escape", async () => {
    render(IconButton, { icon: "plus", label: "New connection" });
    const tip = screen.getByRole("tooltip", { hidden: true });
    expect(tip.textContent).toBe("New connection");
    expect(tip.dataset["open"]).toBe("false");
    await fireEvent.focusIn(screen.getByRole("button", { name: "New connection" }));
    expect(tip.dataset["open"]).toBe("true");
    await fireEvent.keyDown(screen.getByRole("button", { name: "New connection" }), { key: "Escape" });
    expect(tip.dataset["open"]).toBe("false");
  });

  it("delays the tooltip on hover", async () => {
    vi.useFakeTimers();
    render(Tooltip, { text: "Hint", delay: 300, children: button("Target") });
    const tip = screen.getByRole("tooltip", { hidden: true });
    await fireEvent.mouseEnter(screen.getByRole("button", { name: "Target" }).parentElement as HTMLElement);
    expect(tip.dataset["open"]).toBe("false");
    await vi.advanceTimersByTimeAsync(300);
    expect(tip.dataset["open"]).toBe("true");
  });

  it("can drop the tooltip", () => {
    render(IconButton, { icon: "plus", label: "Quiet", tooltip: "" });
    expect(screen.queryByRole("tooltip", { hidden: true })).toBeNull();
  });
});

describe("TextField", () => {
  it("associates the label, hint and error with the input", () => {
    render(TextField, { label: "Host", hint: "DNS name", error: "Required", value: "" });
    const input = screen.getByLabelText("Host");
    expect(input.getAttribute("aria-invalid")).toBe("true");
    const described = input.getAttribute("aria-describedby")?.split(" ") ?? [];
    expect(described.length).toBe(2);
    const texts = described.map((id) => document.getElementById(id)?.textContent);
    expect(texts).toEqual(["DNS name", "Required"]);
    expect(screen.getByRole("alert").textContent).toBe("Required");
  });

  it("has no invalid/described attributes when clean", () => {
    render(TextField, { label: "Name", value: "x" });
    const input = screen.getByLabelText("Name");
    expect(input.hasAttribute("aria-invalid")).toBe(false);
    expect(input.hasAttribute("aria-describedby")).toBe(false);
  });

  it("reveals and hides a password with an accessible toggle", async () => {
    render(TextField, { label: "Password", type: "password", value: "secret" });
    const input = screen.getByLabelText("Password") as HTMLInputElement;
    expect(input.type).toBe("password");
    const toggle = screen.getByRole("button", { name: "Show" });
    expect(toggle.getAttribute("aria-pressed")).toBe("false");
    await fireEvent.click(toggle);
    expect(input.type).toBe("text");
    expect(screen.getByRole("button", { name: "Hide" }).getAttribute("aria-pressed")).toBe("true");
  });

  it("renders string prefix and suffix, and forwards attributes to the input", () => {
    render(TextField, { label: "URL", prefix: "https://", suffix: "TCP", name: "url", "data-autofocus": "" });
    expect(screen.getByText("https://")).toBeTruthy();
    expect(screen.getByText("TCP")).toBeTruthy();
    const input = screen.getByLabelText("URL");
    expect(input.getAttribute("name")).toBe("url");
    expect(input.hasAttribute("data-autofocus")).toBe(true);
  });

  it("disables the input", () => {
    render(TextField, { label: "Off", disabled: true });
    expect((screen.getByLabelText("Off") as HTMLInputElement).disabled).toBe(true);
  });
});

describe("TextArea and Select", () => {
  it("TextArea: label, error and attributes", () => {
    render(TextArea, { label: "Notes", error: "Too short", value: "" });
    const area = screen.getByLabelText("Notes");
    expect(area.tagName).toBe("TEXTAREA");
    expect(area.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByRole("alert").textContent).toBe("Too short");
  });

  it("Select: a native labelled combobox with options and a disabled option", async () => {
    const { component } = render(Select, {
      label: "Protocol",
      value: "ssh",
      options: [
        { value: "ssh", label: "SSH" },
        { value: "rdp", label: "RDP" },
        { value: "web", label: "Web", disabled: true },
      ],
    });
    expect(component).toBeTruthy();
    const select = screen.getByLabelText("Protocol") as HTMLSelectElement;
    expect(select.tagName).toBe("SELECT");
    expect(select.value).toBe("ssh");
    expect(within(select).getAllByRole("option").length).toBe(3);
    expect((within(select).getByRole("option", { name: "Web" }) as HTMLOptionElement).disabled).toBe(true);
    await fireEvent.change(select, { target: { value: "rdp" } });
    expect(select.value).toBe("rdp");
  });

  it("Select: error state", () => {
    render(Select, { label: "Kind", value: "a", options: [{ value: "a", label: "A" }], error: "Bad" });
    expect(screen.getByLabelText("Kind").getAttribute("aria-invalid")).toBe("true");
  });
});

describe("Checkbox, Switch, SegmentedControl", () => {
  it("Checkbox toggles and is labelled", async () => {
    render(Checkbox, { label: "Remember", checked: false });
    const box = screen.getByRole("checkbox", { name: "Remember" }) as HTMLInputElement;
    expect(box.checked).toBe(false);
    await fireEvent.click(box);
    expect(box.checked).toBe(true);
  });

  it("Checkbox supports indeterminate and disabled", () => {
    render(Checkbox, { label: "Some", indeterminate: true, disabled: true });
    const box = screen.getByRole("checkbox", { name: "Some" }) as HTMLInputElement;
    expect(box.indeterminate).toBe(true);
    expect(box.disabled).toBe(true);
  });

  it("Switch has role switch and toggles by click", async () => {
    render(Switch, { label: "Auto-lock", checked: false });
    const sw = screen.getByRole("switch", { name: "Auto-lock" }) as HTMLInputElement;
    expect(sw.checked).toBe(false);
    await fireEvent.click(sw);
    expect(sw.checked).toBe(true);
  });

  it("SegmentedControl is a labelled radio group; selecting fires onchange", async () => {
    const onchange = vi.fn();
    render(SegmentedControl, {
      label: "View",
      value: "list",
      onchange,
      options: [
        { value: "list", label: "List" },
        { value: "grid", label: "Grid" },
        { value: "x", label: "Off", disabled: true },
      ],
    });
    const group = screen.getByRole("radiogroup", { name: "View" });
    const radios = within(group).getAllByRole("radio") as HTMLInputElement[];
    expect(radios.map((r) => r.checked)).toEqual([true, false, false]);
    expect(radios[2]?.disabled).toBe(true);
    await fireEvent.click(radios[1] as HTMLInputElement);
    expect(onchange).toHaveBeenCalledWith("grid");
    expect(radios[1]?.checked).toBe(true);
  });

  it("SegmentedControl icon-only keeps accessible names", () => {
    render(SegmentedControl, {
      label: "Mode",
      value: "a",
      iconOnly: true,
      options: [
        { value: "a", label: "Terminal", icon: "terminal" },
        { value: "b", label: "Desktop", icon: "monitor" },
      ],
    });
    expect(screen.getByRole("radio", { name: "Terminal" })).toBeTruthy();
    expect(screen.getByRole("radio", { name: "Desktop" })).toBeTruthy();
  });
});

describe("Dialog", () => {
  function setup(onclose = vi.fn()) {
    const app = createApp(createMockIpc());
    render(UiHarness, { app, mode: "dialog", onclose });
    return { app, onclose };
  }

  it("is a labelled modal dialog and raises the overlay flag while mounted", () => {
    const { app } = setup();
    const dialog = screen.getByRole("dialog", { name: "Sample" });
    expect(dialog.getAttribute("aria-modal")).toBe("true");
    expect(app.overlay.open).toBe(true);
  });

  it("focuses the first control, traps Tab in both directions", async () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "Sample" });
    const close = within(dialog).getByRole("button", { name: "Close" });
    expect(document.activeElement).toBe(close);
    const save = within(dialog).getByRole("button", { name: "Save" });
    save.focus();
    await fireEvent.keyDown(save, { key: "Tab" });
    expect(document.activeElement).toBe(close);
    await fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(save);
  });

  it("closes on Escape and with the close button", async () => {
    const { onclose } = setup();
    const dialog = screen.getByRole("dialog", { name: "Sample" });
    await fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onclose).toHaveBeenCalledTimes(1);
    await fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    expect(onclose).toHaveBeenCalledTimes(2);
  });

  it("releases the overlay flag and restores focus when unmounted", () => {
    const app = createApp(createMockIpc());
    const view = render(UiHarness, { app, mode: "dialog" });
    expect(app.overlay.open).toBe(true);
    view.unmount();
    expect(app.overlay.open).toBe(false);
  });
});

describe("ConfirmDialog and PromptDialog", () => {
  it("ConfirmDialog: destructive confirm, cancel gets the initial focus, error is an alert", async () => {
    const app = createApp(createMockIpc());
    const onconfirm = vi.fn();
    const oncancel = vi.fn();
    render(UiHarnessConfirm, { app, onconfirm, oncancel });
    const dialog = screen.getByRole("dialog", { name: "Delete it?" });
    expect(document.activeElement).toBe(within(dialog).getByRole("button", { name: "Cancel" }));
    const confirm = within(dialog).getByRole("button", { name: "Delete" });
    expect(confirm.classList.contains("destructive")).toBe(true);
    expect(within(dialog).getByRole("alert").textContent).toContain("Failed");
    await fireEvent.click(confirm);
    expect(onconfirm).toHaveBeenCalled();
  });

  it("PromptDialog: validates and shows the error next to the field", async () => {
    const app = createApp(createMockIpc());
    const onsubmit = vi.fn(() => null);
    render(UiHarnessPrompt, { app, onsubmit, oncancel: vi.fn() });
    const dialog = screen.getByRole("dialog", { name: "New folder" });
    const field = within(dialog).getByLabelText("Name") as HTMLInputElement;
    expect(document.activeElement).toBe(field);
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).not.toHaveBeenCalled();
    expect(within(dialog).getByRole("alert")).toBeTruthy();
    expect(field.getAttribute("aria-invalid")).toBe("true");
    await fireEvent.input(field, { target: { value: "Prod" } });
    await fireEvent.submit(dialog.querySelector("form") as HTMLFormElement);
    expect(onsubmit).toHaveBeenCalledWith("Prod");
  });
});

describe("ContextMenu", () => {
  const items = (calls: string[]): MenuItem[] => [
    { label: "Open", action: () => calls.push("open"), shortcut: "⏎" },
    { label: "Disabled", action: () => calls.push("disabled"), disabled: true },
    { label: "Delete", action: () => calls.push("delete"), danger: true, separatorBefore: true },
  ];

  it("renders menu items with shortcut hint, separator, danger and disabled state", () => {
    const app = createApp(createMockIpc());
    render(UiHarness, { app, mode: "menu", items: items([]) });
    const menu = screen.getByRole("menu", { name: "Sample menu" });
    expect(within(menu).getAllByRole("menuitem").length).toBe(3);
    expect(within(menu).getByRole("separator")).toBeTruthy();
    expect(within(menu).getByText("⏎")).toBeTruthy();
    expect((within(menu).getByRole("menuitem", { name: "Disabled" }) as HTMLButtonElement).disabled).toBe(true);
    expect(within(menu).getByRole("menuitem", { name: "Delete" }).classList.contains("danger")).toBe(true);
    expect(app.overlay.open).toBe(true);
  });

  it("moves with arrows (skipping disabled), Home/End, runs on click, closes on Escape", async () => {
    const calls: string[] = [];
    const onclose = vi.fn();
    const app = createApp(createMockIpc());
    render(UiHarness, { app, mode: "menu", items: items(calls), onclose });
    const menu = screen.getByRole("menu");
    const open = within(menu).getByRole("menuitem", { name: "Open" });
    const del = within(menu).getByRole("menuitem", { name: "Delete" });
    expect(document.activeElement).toBe(open);
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(del);
    await fireEvent.keyDown(menu, { key: "ArrowDown" });
    expect(document.activeElement).toBe(open);
    await fireEvent.keyDown(menu, { key: "End" });
    expect(document.activeElement).toBe(del);
    await fireEvent.keyDown(menu, { key: "Home" });
    expect(document.activeElement).toBe(open);
    await fireEvent.click(del);
    expect(calls).toEqual(["delete"]);
    await fireEvent.keyDown(menu, { key: "Escape" });
    expect(onclose).toHaveBeenCalled();
  });
});

describe("Toasts", () => {
  it("shows notices with the right live roles and dismisses them", async () => {
    const app = createApp(createMockIpc());
    render(UiHarness, { app, mode: "toasts" });
    app.notices.push("error", "Could not connect", "refused");
    app.notices.push("info", "Saved");
    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("status").textContent).toContain("Saved");
    expect(screen.getByRole("region", { name: "Notifications" })).toBeTruthy();
    await fireEvent.click(within(screen.getByRole("alert")).getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("ListRow", () => {
  it("passes role and aria attributes through, marks selection, renders slots", async () => {
    const onclick = vi.fn();
    render(ListRow, {
      role: "option",
      "aria-selected": true,
      selected: true,
      depth: 2,
      onclick,
      children: text("prod-db-01"),
      leading: text("L"),
      trailing: text("T"),
      secondary: text("admin@host"),
    });
    const row = screen.getByRole("option", { name: /prod-db-01/ });
    expect(row.getAttribute("aria-selected")).toBe("true");
    expect(row.classList.contains("selected")).toBe(true);
    expect(row.style.getPropertyValue("--depth")).toBe("2");
    expect(row.textContent).toContain("admin@host");
    await fireEvent.click(row);
    expect(onclick).toHaveBeenCalled();
  });
});

describe("Banner, Badge, EmptyState, Spinner, Separator, Kbd", () => {
  it("Banner: danger and warning are alerts, info is a status, dismiss works", async () => {
    const ondismiss = vi.fn();
    const { unmount } = render(Banner, { variant: "danger", title: "Host key changed", children: text("body") });
    expect(screen.getByRole("alert").textContent).toContain("Host key changed");
    unmount();
    const w = render(Banner, { variant: "warning", children: text("careful") });
    expect(screen.getByRole("alert")).toBeTruthy();
    w.unmount();
    render(Banner, { variant: "info", children: text("fyi"), ondismiss });
    await fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.getByRole("status")).toBeTruthy();
    expect(ondismiss).toHaveBeenCalled();
  });

  it("Banner: role can be overridden", () => {
    render(Banner, { variant: "danger", role: "note", children: text("x") });
    expect(screen.getByRole("note")).toBeTruthy();
  });

  it("Badge renders its variant", () => {
    render(Badge, { variant: "success", children: text("Live") });
    expect(screen.getByText("Live").parentElement?.classList.contains("success")).toBe(true);
  });

  it("EmptyState: heading, text and action", () => {
    render(EmptyState, { icon: "plug", title: "No connections", text: "Create one.", action: button("Add") });
    expect(screen.getByRole("heading", { name: "No connections" })).toBeTruthy();
    expect(screen.getByText("Create one.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Add" })).toBeTruthy();
  });

  it("Spinner: silent by default, a status when labelled", () => {
    const { container, unmount } = render(Spinner, {});
    expect(container.querySelector(".spinner")?.getAttribute("aria-hidden")).toBe("true");
    unmount();
    render(Spinner, { label: "Loading" });
    expect(screen.getByRole("status", { name: "Loading" })).toBeTruthy();
  });

  it("Separator: decorative by default, semantic on request", () => {
    const { container, unmount } = render(Separator, {});
    expect(container.querySelector("[role=none]")).not.toBeNull();
    unmount();
    render(Separator, { decorative: false, orientation: "vertical" });
    expect(screen.getByRole("separator").getAttribute("aria-orientation")).toBe("vertical");
  });

  it("Kbd renders a kbd element", () => {
    const { container } = render(Kbd, { children: text("⌘N") });
    expect(container.querySelector("kbd")?.textContent).toBe("⌘N");
  });
});

describe("Toolbar", () => {
  it("is a labelled toolbar; arrows, Home and End move focus between buttons", async () => {
    render(Toolbar, {
      label: "Tools",
      start: createRawSnippet(() => ({
        render: () => `<span><button type="button">A</button><button type="button">B</button></span>`,
      })),
      end: createRawSnippet(() => ({ render: () => `<button type="button">C</button>` })),
    });
    const bar = screen.getByRole("toolbar", { name: "Tools" });
    const [a, b, c] = within(bar).getAllByRole("button");
    (a as HTMLElement).focus();
    await fireEvent.keyDown(a as HTMLElement, { key: "ArrowRight" });
    expect(document.activeElement).toBe(b);
    await fireEvent.keyDown(b as HTMLElement, { key: "End" });
    expect(document.activeElement).toBe(c);
    await fireEvent.keyDown(c as HTMLElement, { key: "ArrowRight" });
    expect(document.activeElement).toBe(a);
    await fireEvent.keyDown(a as HTMLElement, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(c);
    await fireEvent.keyDown(c as HTMLElement, { key: "Home" });
    expect(document.activeElement).toBe(a);
  });
});

describe("FormRow and FormSection", () => {
  it("FormRow associates its label with the control through `for`", () => {
    render(FormRow, {
      label: "Host",
      for: "host-input",
      hint: "DNS name",
      error: "Required",
      children: createRawSnippet(() => ({ render: () => `<input id="host-input" />` })),
    });
    expect(screen.getByLabelText("Host").id).toBe("host-input");
    expect(screen.getByRole("alert").textContent).toBe("Required");
  });

  it("FormRow without `for` is a labelled group", () => {
    render(FormRow, { label: "Options", children: text("x") });
    expect(screen.getByRole("group", { name: "Options" })).toBeTruthy();
  });

  it("FormSection is a labelled region with a heading", () => {
    render(FormSection, { title: "General", description: "Basics", children: text("rows") });
    expect(screen.getByRole("heading", { name: "General" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "General" })).toBeTruthy();
  });
});

describe("Icon, KindIcon, Logo, TitlebarDragRegion", () => {
  it("Icon is decorative by default and an image when labelled", () => {
    const { container, unmount } = render(Icon, { name: "plus" });
    expect(container.querySelector("svg")?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelector("svg")?.getAttribute("width")).toBe("16");
    unmount();
    render(Icon, { name: "plus", label: "Add", size: 20 });
    const img = screen.getByRole("img", { name: "Add" });
    expect(img.getAttribute("width")).toBe("20");
  });

  it("Icon uses a consistent stroke", () => {
    const { container } = render(Icon, { name: "search" });
    expect(container.querySelector("svg")?.getAttribute("stroke-width")).toBe("1.75");
  });

  it("KindIcon draws every kind with a round-capped single stroke", () => {
    for (const kind of ["folder", "localShell", "ssh", "rdp", "vnc", "web"] as const) {
      const { container, unmount } = render(KindIcon, { kind });
      const svg = container.querySelector("svg") as SVGElement;
      expect(svg.getAttribute("data-kind")).toBe(kind);
      expect(svg.getAttribute("stroke-linecap")).toBe("round");
      expect(svg.getAttribute("aria-hidden")).toBe("true");
      expect(svg.querySelectorAll("path, rect, circle").length).toBeGreaterThan(0);
      unmount();
    }
  });

  it("KindIcon folder differs when open", () => {
    const closed = render(KindIcon, { kind: "folder" });
    const closedHtml = closed.container.innerHTML;
    closed.unmount();
    const open = render(KindIcon, { kind: "folder", open: true });
    expect(open.container.innerHTML).not.toBe(closedHtml);
  });

  it("Logo is decorative by default and follows currentColor", () => {
    const { container, unmount } = render(Logo, { size: 32 });
    const svg = container.querySelector("svg") as SVGElement;
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("width")).toBe("32");
    expect(container.querySelector("path")?.getAttribute("stroke")).toBe("currentColor");
    unmount();
    render(Logo, { label: "Portolano" });
    expect(screen.getByRole("img", { name: "Portolano" })).toBeTruthy();
  });

  it("TitlebarDragRegion carries the Tauri drag attribute and is hidden from assistive tech", () => {
    const { container } = render(TitlebarDragRegion, {});
    const region = container.querySelector("[data-tauri-drag-region]") as HTMLElement;
    expect(region).not.toBeNull();
    expect(region.getAttribute("aria-hidden")).toBe("true");
  });
});

describe("Radio and RadioGroup", () => {
  const options = [
    { value: "a", label: "Alpha", hint: "First choice" },
    { value: "b", label: "Beta", hint: "Second choice" },
    { value: "c", label: "Gamma", disabled: true },
  ];

  it("is a labelled radio group of native radios, each with its label and hint", () => {
    render(RadioGroup, { label: "Letter", options, value: "b", idPrefix: "pick" });
    const group = screen.getByRole("radiogroup", { name: "Letter" });
    const radios = within(group).getAllByRole("radio") as HTMLInputElement[];
    expect(radios.map((r) => r.value)).toEqual(["a", "b", "c"]);
    expect(radios.map((r) => r.id)).toEqual(["pick-a", "pick-b", "pick-c"]);
    expect(new Set(radios.map((r) => r.name)).size).toBe(1);
    expect(radios.find((r) => r.checked)?.value).toBe("b");
    const alpha = within(group).getByLabelText("Alpha");
    expect(alpha.getAttribute("aria-describedby")).toBe("pick-a-hint");
    expect(document.getElementById("pick-a-hint")?.textContent).toBe("First choice");
    expect(radios[2]?.disabled).toBe(true);
    expect(radios[2]?.hasAttribute("aria-describedby")).toBe(false);
  });

  it("selects on click and on arrow keys, one at a time", async () => {
    render(RadioGroup, { label: "Letter", options, value: "a" });
    const [a, b] = screen.getAllByRole("radio") as HTMLInputElement[];
    await fireEvent.click(b as HTMLInputElement);
    expect(b?.checked).toBe(true);
    expect(a?.checked).toBe(false);
  });

  it("disables every option when the group is disabled", () => {
    render(RadioGroup, { label: "Letter", options, value: "a", disabled: true });
    expect((screen.getAllByRole("radio") as HTMLInputElement[]).every((r) => r.disabled)).toBe(true);
  });

  it("Radio stands alone with a label, a hint and the forwarded attributes", () => {
    render(Radio, { value: "x", label: "Single", hint: "Alone", name: "solo", "data-testid": "solo" });
    const input = screen.getByLabelText("Single") as HTMLInputElement;
    expect(input.type).toBe("radio");
    expect(input.name).toBe("solo");
    expect(input.getAttribute("aria-describedby")).toBe(`${input.id}-hint`);
    expect(input.getAttribute("data-testid")).toBe("solo");
  });
});

describe("monospace fields and the exposed search input", () => {
  it("TextField and TextArea switch to monospace on request only", () => {
    render(TextField, { label: "Host", value: "", monospace: true });
    render(TextField, { label: "Name", value: "" });
    render(TextArea, { label: "Key", value: "", monospace: true });
    render(TextArea, { label: "Notes", value: "" });
    expect(screen.getByLabelText("Host").classList.contains("monospace")).toBe(true);
    expect(screen.getByLabelText("Name").classList.contains("monospace")).toBe(false);
    expect(screen.getByLabelText("Key").classList.contains("monospace")).toBe(true);
    expect(screen.getByLabelText("Notes").classList.contains("monospace")).toBe(false);
  });

  it("TextField hands out its input (bind:input) so a parent can focus it", async () => {
    const app = createApp(createMockIpc());
    render(UiHarnessHero, { app });
    const field = screen.getByLabelText("Query") as HTMLInputElement;
    expect(field.type).toBe("search");
    screen.getByRole("button", { name: "focus-field" }).focus();
    await fireEvent.click(screen.getByRole("button", { name: "focus-field" }));
    expect(document.activeElement).toBe(field);
  });
});

describe("Dialog hero and widths, ListRow states, Toolbar flush", () => {
  it("renders the hero strip between the title row and the body, outside the controls", () => {
    render(UiHarnessHero, { app: createApp(createMockIpc()), size: "sm" });
    const dialog = screen.getByRole("dialog", { name: "With hero" });
    const strip = within(dialog).getByTestId("hero-strip");
    const hero = strip.parentElement as HTMLElement;
    expect(hero.classList.contains("hero")).toBe(true);
    expect(hero.previousElementSibling?.tagName).toBe("HEADER");
    expect(hero.nextElementSibling?.classList.contains("body")).toBe(true);
    // Presets come from classes (token widths), not inline styles.
    expect(dialog.classList.contains("sm")).toBe(true);
    expect(dialog.getAttribute("style") ?? "").not.toContain("width");
  });

  it("an explicit width still overrides the preset", () => {
    render(UiHarnessHero, { app: createApp(createMockIpc()), size: "lg", width: 500 });
    expect(screen.getByRole("dialog").getAttribute("style")).toContain("500px");
  });

  it("has no hero element without the slot", () => {
    render(UiHarness, { app: createApp(createMockIpc()), mode: "dialog" });
    expect(screen.getByRole("dialog").querySelector(".hero")).toBeNull();
  });

  it("ListRow shows drop-target, dragging and comfortable density as classes", () => {
    render(ListRow, {
      role: "option",
      dropTarget: true,
      dragging: true,
      density: "comfortable",
      children: text("row"),
    });
    const row = screen.getByRole("option");
    expect(row.classList.contains("drop-target")).toBe(true);
    expect(row.classList.contains("dragging")).toBe(true);
    expect(row.classList.contains("comfortable")).toBe(true);
  });

  it("ListRow carries none of those by default", () => {
    render(ListRow, { role: "option", children: text("row") });
    const row = screen.getByRole("option");
    for (const name of ["drop-target", "dragging", "comfortable", "compact"]) {
      expect(row.classList.contains(name), name).toBe(false);
    }
    expect(row.classList.contains("regular")).toBe(true);
  });

  it("Toolbar can be flush", () => {
    render(Toolbar, { label: "Flat", flush: true, children: button("A") });
    expect(screen.getByRole("toolbar", { name: "Flat" }).classList.contains("flush")).toBe(true);
  });
});
