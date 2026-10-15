import { describe, expect, it } from "vitest";
import { cn } from "./cn";

describe("cn with the custom tokens", () => {
  it("keeps a text colour and a text size together", () => {
    expect(cn("text-muted", "text-12")).toBe("text-muted text-12");
  });
  it("dedupes the row spacing step against the numeric scale", () => {
    expect(cn("h-row", "h-4")).toBe("h-4");
  });
  it("treats rounded-ctl as a radius", () => {
    expect(cn("rounded-ctl", "rounded-none")).toBe("rounded-none");
  });
  it("treats shadow-float as a shadow", () => {
    expect(cn("shadow-float", "shadow-none")).toBe("shadow-none");
  });
});
