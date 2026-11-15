import { cleanup, render } from "@testing-library/svelte";
import { afterEach, describe, expect, it } from "vitest";

import StrengthHint from "./StrengthHint.svelte";

afterEach(cleanup);

const filled = (container: HTMLElement): number => container.querySelectorAll(".step.on").length;

describe("StrengthHint", () => {
  it("fills no step and shows no text for an empty password", () => {
    const { container } = render(StrengthHint, { props: { password: "" } });
    expect(filled(container)).toBe(0);
    expect(container.querySelector(".label")?.textContent).toBe("");
  });

  it("fills more steps as the password gets stronger and always says it in text", () => {
    const weak = render(StrengthHint, { props: { password: "abc" } });
    expect(filled(weak.container)).toBe(1);
    expect(weak.container.querySelector(".label")?.textContent).toBe("Strength: weak");
    cleanup();
    const strong = render(StrengthHint, { props: { password: "correct-Horse-battery-9" } });
    expect(filled(strong.container)).toBe(4);
    expect(strong.container.querySelector(".label")?.textContent).toBe("Strength: strong");
  });

  it("keeps the meter out of the accessibility tree and the label a live region", () => {
    const { container } = render(StrengthHint, { props: { password: "abc" } });
    expect(container.querySelector(".meter")?.getAttribute("aria-hidden")).toBe("true");
    expect(container.querySelector(".label")?.getAttribute("aria-live")).toBe("polite");
  });
});
