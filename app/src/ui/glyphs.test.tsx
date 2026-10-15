import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EntityGlyph } from "./glyphs";

describe("glyphs", () => {
  it.each(["namespace", "queue", "topic", "subscription", "rule"] as const)("%s is a decorative 16px-grid svg", (kind) => {
    const { container } = render(<EntityGlyph kind={kind} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 16 16");
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("stroke")).toBe("currentColor");
  });
});
