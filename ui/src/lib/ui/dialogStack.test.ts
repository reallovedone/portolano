// Dialogs that open over each other: the newest is on top, focused, handles Escape first, and
// the one below is inert, keeps its state and gets its focus back when the top one closes.

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import { createMockIpc } from "../ipc/mock";
import { createApp } from "../state/app";
import DialogStackHarness from "./DialogStackHarness.svelte";

afterEach(cleanup);

function mount() {
  const app = createApp(createMockIpc());
  render(DialogStackHarness, { props: { app } });
  return app;
}

const backdropOf = (dialog: HTMLElement): HTMLElement => dialog.parentElement as HTMLElement;
const depth = (dialog: HTMLElement): number => Number(backdropOf(dialog).dataset.stackDepth);
const isInert = (dialog: HTMLElement): boolean => backdropOf(dialog).hasAttribute("inert");

async function openInner(): Promise<HTMLElement> {
  await fireEvent.click(screen.getByRole("button", { name: "Open inner" }));
  return screen.findByRole("dialog", { name: "Inner" });
}

describe("dialog stack", () => {
  it("puts the newest dialog on top whatever the DOM order, and covers the one below", async () => {
    mount();
    const outer = await screen.findByRole("dialog", { name: "Outer" });
    expect(isInert(outer)).toBe(false);
    const inner = await openInner();
    // The inner dialog comes first in the DOM, yet it is the one above.
    expect(depth(outer)).toBe(0);
    expect(depth(inner)).toBe(1);
    expect(isInert(outer)).toBe(true);
    expect(isInert(inner)).toBe(false);
    expect(backdropOf(inner).style.getPropertyValue("z-index")).toContain("+ 1");
    // Everything inside the covered dialog is inert (no pointer, focus or keyboard).
    expect(screen.getByRole("button", { name: "Open inner" }).closest("[inert]")).not.toBeNull();
  });

  it("focuses the new dialog, traps Tab inside it, and gives the focus back on close", async () => {
    mount();
    const outer = await screen.findByRole("dialog", { name: "Outer" });
    const opener = within(outer).getByRole("button", { name: "Open inner" });
    opener.focus();
    const inner = await openInner();
    await waitFor(() => expect(inner.contains(document.activeElement)).toBe(true));
    expect(document.activeElement).toBe(within(inner).getByLabelText("Inner field"));

    // Tab never leaves the top dialog.
    for (let i = 0; i < 6; i += 1) {
      await fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Tab" });
      if (!inner.contains(document.activeElement)) break;
    }
    expect(inner.contains(document.activeElement)).toBe(true);

    await fireEvent.keyDown(inner, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull());
    expect(isInert(outer)).toBe(false);
    expect(document.activeElement).toBe(opener);
  });

  it("keeps the state of the dialog below", async () => {
    mount();
    const outer = await screen.findByRole("dialog", { name: "Outer" });
    await fireEvent.input(within(outer).getByLabelText("Outer field"), { target: { value: "half filled" } });
    const inner = await openInner();
    await fireEvent.keyDown(inner, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull());
    const again = screen.getByRole("dialog", { name: "Outer" });
    expect((within(again).getByLabelText("Outer field") as HTMLInputElement).value).toBe("half filled");
  });

  it("handles Escape from the top dialog first, one level at a time", async () => {
    mount();
    await screen.findByRole("dialog", { name: "Outer" });
    const inner = await openInner();
    await fireEvent.click(within(inner).getByRole("button", { name: "Open third" }));
    const third = await screen.findByRole("dialog", { name: "Third" });
    expect(depth(third)).toBe(2);
    expect(isInert(inner)).toBe(true);

    await fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Third" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Inner" })).toBeTruthy();
    expect(isInert(inner)).toBe(false);
    expect(inner.contains(document.activeElement)).toBe(true);

    await fireEvent.keyDown(document.activeElement as HTMLElement, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Outer" })).toBeTruthy();
  });

  it("still closes the top dialog on Escape when focus has drifted outside it", async () => {
    mount();
    await screen.findByRole("dialog", { name: "Outer" });
    await openInner();
    (document.activeElement as HTMLElement).blur();
    await fireEvent.keyDown(document.body, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull());
    expect(screen.getByRole("dialog", { name: "Outer" })).toBeTruthy();
  });

  it("closes a pop-up of the dialog below when another dialog opens over it", async () => {
    mount();
    const outer = await screen.findByRole("dialog", { name: "Outer" });
    const combo = within(outer).getByRole("combobox");
    await fireEvent.click(combo);
    expect(document.querySelector("[role=listbox]")).not.toBeNull();
    await fireEvent.click(within(outer).getByRole("button", { name: "Open inner" }));
    await screen.findByRole("dialog", { name: "Inner" });
    await waitFor(() => expect(document.querySelector("[role=listbox]")).toBeNull());
  });

  it("keeps the overlay flag while any dialog is left", async () => {
    const app = mount();
    await screen.findByRole("dialog", { name: "Outer" });
    const inner = await openInner();
    expect(app.overlay.open).toBe(true);
    await fireEvent.keyDown(inner, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Inner" })).toBeNull());
    expect(app.overlay.open).toBe(true);
  });
});
