import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { MAX_LINES } from "./format";
import { CodeView } from "./CodeView";
import { HexView } from "./HexView";

describe("CodeView", () => {
  it("renders only visible lines of a long body, with line numbers", () => {
    const text = Array.from({ length: 50_000 }, (_, i) => `line ${i + 1}`).join("\n");
    render(<CodeView text={text} label="Body" height={400} />);
    const view = screen.getByRole("region", { name: "Body" });
    expect(within(view).getByText("line 1")).toBeInTheDocument();
    expect(view.querySelectorAll("[data-line]").length).toBeLessThan(120);
  });
  it("cuts very long lines with a marker", () => {
    render(<CodeView text={"x".repeat(30_000)} label="Body" height={100} />);
    expect(screen.getByText(/line shortened/)).toBeInTheDocument();
  });
  it("renders an empty body as one empty line without throwing", () => {
    render(<CodeView text="" label="Body" height={100} />);
    expect(screen.getByRole("region", { name: "Body" }).querySelectorAll("[data-line]")).toHaveLength(1);
  });
  it("says how many lines past the display limit are left out", () => {
    const text = "x\n".repeat(MAX_LINES + 40) + "x"; // MAX_LINES + 41 lines
    render(<CodeView text={text} label="Body" height={100} />);
    const view = screen.getByRole("region", { name: "Body" });
    // MAX_LINES rows plus the marker row.
    expect(parseInt((view.firstElementChild as HTMLElement).style.height, 10)).toBe((MAX_LINES + 1) * 16);
    act(() => {
      view.scrollTop = (MAX_LINES + 1) * 16;
      fireEvent.scroll(view);
    });
    expect(screen.getByText(/41 more lines not shown/)).toBeInTheDocument();
  });
  it("is focusable so the keyboard can scroll it", () => {
    render(<CodeView text="a" label="Body" height={100} />);
    expect(screen.getByRole("region", { name: "Body" })).toHaveAttribute("tabindex", "0");
  });
});

describe("HexView", () => {
  it("renders offset, hex and text columns", () => {
    render(<HexView bytes={new Uint8Array([0x48, 0x69])} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" })).toHaveTextContent("00000000");
    expect(screen.getByText("48 69")).toBeInTheDocument();
    expect(screen.getByText("|Hi|")).toBeInTheDocument();
  });
  it("renders only visible rows of a 4 MiB body and survives an empty one", () => {
    const { unmount } = render(<HexView bytes={new Uint8Array(4 * 1024 * 1024)} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" }).querySelectorAll("[data-row]").length).toBeLessThan(120);
    unmount();
    render(<HexView bytes={new Uint8Array()} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" }).querySelectorAll("[data-row]")).toHaveLength(0);
  });
});
