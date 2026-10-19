import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { MAX_LINES } from "./format";
import { CodeView } from "./CodeView";
import { HexView } from "./HexView";
import { BodyState } from "./BodyState";

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
    const notice = screen.getByText(/Line shortened\. Copy the body for all of it\./);
    // Pinned to the right edge of the visible area, so it shows without scrolling sideways.
    expect(notice.className).toContain("sticky");
    expect(notice.className).toContain("right-0");
    expect(notice.className).toContain("bg-content");
    expect(notice.textContent).not.toContain("…");
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
    expect(screen.getByText("41 more lines not shown. Copy the body for all of it.")).toBeInTheDocument();
  });
  it("takes the line limit and a caller's notice", () => {
    const text = Array.from({ length: 30 }, (_, i) => `l${i}`).join("\n");
    const { unmount } = render(<CodeView text={text} label="Body" height={400} maxLines={10} />);
    expect(screen.getByText("20 more lines not shown. Copy the body for all of it.")).toBeInTheDocument();
    unmount();
    render(<CodeView text={text} label="Body" height={400} maxLines={10} notice="Showing the first 4 MiB." />);
    expect(screen.getByText("Showing the first 4 MiB.")).toBeInTheDocument();
    expect(screen.queryByText(/more lines not shown/)).toBeNull();
  });
  it("shows a caller's notice under a body that is not line-capped", () => {
    render(<CodeView text="a" label="Body" height={100} notice="Truncated at 4 MiB." />);
    expect(screen.getByText("Truncated at 4 MiB.")).toBeInTheDocument();
  });
  it("is focusable so the keyboard can scroll it", () => {
    render(<CodeView text="a" label="Body" height={100} />);
    expect(screen.getByRole("region", { name: "Body" })).toHaveAttribute("tabindex", "0");
  });
});

describe("HexView", () => {
  it("renders offset, hex and text columns", () => {
    render(<HexView bytes={new Uint8Array([0x48, 0x69])} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" })).toHaveTextContent("000000");
    expect(screen.getByText("48 69")).toBeInTheDocument();
    expect(screen.getByText("|Hi|")).toBeInTheDocument();
  });
  it("has a sticky header with the offset, byte columns and text, and dims zero bytes", () => {
    render(<HexView bytes={new Uint8Array([0x48, 0x00, 0x69])} label="Hex" />);
    const header = screen.getByText("Offset").parentElement!;
    expect(header.className).toContain("sticky");
    expect(header).toHaveTextContent("00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f");
    expect(header).toHaveTextContent("Text");
    const row = screen.getByRole("region", { name: "Hex" }).querySelector("[data-row]")!;
    const zero = within(row as HTMLElement).getByText("00");
    expect(zero.className).toContain("text-faint");
    expect(header.querySelector("[data-row]")).toBeNull();
  });
  it("holds 16 bytes per row in a wide view and 8 in a narrow one, and follows a resize", () => {
    let width = 700;
    let resize: () => void = () => {};
    vi.stubGlobal("ResizeObserver", class { constructor(cb: () => void) { resize = cb; } observe() {} disconnect() {} unobserve() {} });
    const client = vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => width);
    const rect = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(() => ({ width: 66, height: 12 }) as DOMRect);
    try {
      render(<HexView bytes={Uint8Array.from({ length: 40 }, (_, i) => i)} label="Hex" />);
      const region = screen.getByRole("region", { name: "Hex" });
      expect(region).toHaveTextContent("08 09 0a 0b 0c 0d 0e 0f");
      expect(region.querySelectorAll("[data-row]")).toHaveLength(3);
      width = 400;
      act(() => resize());
      expect(screen.getByText("Offset").parentElement).toHaveTextContent("00 01 02 03 04 05 06 07Text");
      expect(region.querySelectorAll("[data-row]")).toHaveLength(5);
      expect(region).toHaveTextContent("000020");
    } finally {
      client.mockRestore();
      rect.mockRestore();
      vi.unstubAllGlobals();
    }
  });
  it("shows a caller's notice as the last row", () => {
    render(<HexView bytes={new Uint8Array(4)} label="Hex" notice="Showing the first 4 MiB." />);
    expect(screen.getByText("Showing the first 4 MiB.")).toBeInTheDocument();
  });
  it("renders only visible rows of a 4 MiB body and survives an empty one", () => {
    const { unmount } = render(<HexView bytes={new Uint8Array(4 * 1024 * 1024)} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" }).querySelectorAll("[data-row]").length).toBeLessThan(120);
    unmount();
    render(<HexView bytes={new Uint8Array()} label="Hex" />);
    expect(screen.getByRole("region", { name: "Hex" }).querySelectorAll("[data-row]")).toHaveLength(0);
  });
});

describe("BodyState", () => {
  it("says a body is empty, binary or too large to format, in one muted line", () => {
    render(<><BodyState kind="empty" /><BodyState kind="binary" /><BodyState kind="too-large" /></>);
    expect(screen.getByText("Empty body (0 bytes)")).toBeInTheDocument();
    expect(screen.getByText("Not valid UTF-8, shown as hex")).toBeInTheDocument();
    expect(screen.getByText("Too large to format; showing raw text.")).toBeInTheDocument();
    expect(screen.getByText("Empty body (0 bytes)").className).toContain("text-muted");
  });
});
