// Smoke test of the design gallery: it mounts and shows the pieces added in D3, so a broken
// gallery is caught by `pnpm test` and not only by a human opening /?gallery.
import { cleanup, render, screen, within } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import Gallery from "./Gallery.svelte";

afterEach(cleanup);

describe("Gallery", () => {
  it("mounts with the terminal palette, letterbox, radios and the new component states", () => {
    const { container } = render(Gallery);
    expect(screen.getByText("Portolano design gallery")).toBeTruthy();
    // 16 ANSI colours in each of the two themes, drawn on the terminal background.
    expect(container.querySelectorAll(".ansi-row").length).toBe(32);
    expect(container.querySelector(".letterbox")).not.toBeNull();
    const group = screen.getByRole("radiogroup", { name: "Credential mode" });
    expect(within(group).getAllByRole("radio")).toHaveLength(3);
    expect(container.querySelector(".drop-target")).not.toBeNull();
    expect(container.querySelector(".dragging")).not.toBeNull();
    expect(container.querySelector(".comfortable")).not.toBeNull();
    expect(screen.getByRole("toolbar", { name: /^Flush toolbar/ }).classList.contains("flush")).toBe(true);
    expect(container.querySelector("input.monospace")).not.toBeNull();
    expect(container.querySelector("textarea.monospace")).not.toBeNull();
  });
});
