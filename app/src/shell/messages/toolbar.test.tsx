import { act, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/ui";
import { COMPACT_BELOW, Toolbar } from "./Toolbar";

let width = 1000;
let resize: () => void = () => {};

function stubWidth() {
  vi.stubGlobal("ResizeObserver", class { constructor(cb: () => void) { resize = cb; } observe() {} disconnect() {} unobserve() {} });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => width);
}
afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const bar = () => (
  <TooltipProvider>
    <Toolbar
      from={{ kind: "start" }} onFrom={() => {}} topN={100} onTopN={() => {}} filter={{}} filterCount={0} filterOpen={false} onClearFilter={() => {}}
      onToggleFilter={() => {}} columns={["seq"]} offered={["seq"]} onColumns={() => {}} onResetColumns={() => {}} propertyNames={[]}
      streaming={false} onStop={() => {}} onRefresh={() => {}} exportMenu={() => null}
    />
  </TooltipProvider>
);

describe("Toolbar", () => {
  it("shows icons alone below the threshold, keeping each name, and does not remount the buttons when it crosses it", () => {
    stubWidth();
    render(bar());
    const filter = screen.getByRole("button", { name: "Filter" });
    expect(filter).toHaveTextContent("Filter");
    expect(screen.getByText("From")).toBeInTheDocument();
    width = COMPACT_BELOW - 1;
    act(() => resize());
    // The same element: React kept the button, only its label and the "From" text changed.
    expect(screen.getByRole("button", { name: "Filter" })).toBe(filter);
    expect(filter).not.toHaveTextContent("Filter");
    expect(screen.queryByText("From")).toBeNull();
    expect(screen.getByRole("button", { name: "Columns" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Peek again" })).not.toHaveTextContent("Peek again");
    width = 1000;
    act(() => resize());
    expect(screen.getByRole("button", { name: "Filter" })).toBe(filter);
    expect(filter).toHaveTextContent("Filter");
  });
});
