import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ColumnDef } from "@tanstack/react-table";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { DataGrid } from "./DataGrid";

interface Msg { seq: number; subject: string; size: string }
const columns: ColumnDef<Msg, unknown>[] = [
  { accessorKey: "seq", header: "Seq", meta: { mono: true, width: "64px" } },
  { accessorKey: "subject", header: "Subject", meta: { width: "minmax(140px, 1fr)" } },
  { accessorKey: "size", header: "Size", meta: { align: "end", width: "60px" } },
];
const rows = (n: number): Msg[] => Array.from({ length: n }, (_, i) => ({ seq: 48000 + i, subject: `Subject ${i}`, size: "1.8 KB" }));

function Harness({ data = rows(20), onActivate = () => {}, ...rest }: { data?: Msg[]; onActivate?: (id: string) => void; loading?: boolean; empty?: React.ReactNode; error?: React.ReactNode; rowHeight?: number }) {
  const [sel, setSel] = useState<Set<string>>(new Set());
  const [focus, setFocus] = useState<string | null>(null);
  return (
    <DataGrid
      label="Dead-lettered messages"
      columns={columns}
      rows={data}
      getRowId={(r) => String(r.seq)}
      selectedIds={sel}
      focusId={focus}
      onSelectionChange={(ids, f) => { setSel(ids); setFocus(f); }}
      onActivate={onActivate}
      rowHeight={22}
      initialRect={{ width: 800, height: 300 }}
      {...rest}
    />
  );
}

const selectedRows = () => screen.getAllByRole("row").filter((r) => r.getAttribute("aria-selected") === "true");

describe("DataGrid", () => {
  it("renders headers and only visible rows of 10,000", () => {
    render(<Harness data={rows(10_000)} />);
    const grid = screen.getByRole("grid", { name: "Dead-lettered messages" });
    expect(within(grid).getAllByRole("columnheader").map((h) => h.textContent)).toEqual(["Seq", "Subject", "Size"]);
    expect(within(grid).getAllByRole("row").length).toBeLessThan(80);
  });

  it("right-aligns end columns and uses mono where asked", () => {
    render(<Harness />);
    const cells = within(screen.getAllByRole("row")[1]).getAllByRole("gridcell");
    expect(cells[0].className).toContain("font-mono");
    expect(cells[2].className).toContain("text-right");
  });

  it("moves, extends and activates with the keyboard", async () => {
    const onActivate = vi.fn();
    render(<Harness onActivate={onActivate} />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("jj");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48001"]);
    await userEvent.keyboard("{Shift>}{ArrowDown}{ArrowDown}{/Shift}");
    expect(selectedRows()).toHaveLength(3);
    await userEvent.keyboard("{Enter}");
    expect(onActivate).toHaveBeenCalledWith("48003");
  });

  it("toggles with mod-click and ranges with shift-click", async () => {
    // One user instance so held modifier keys apply to the clicks.
    const user = userEvent.setup();
    render(<Harness />);
    const rowEls = () => screen.getAllByRole("row").slice(1);
    await user.click(rowEls()[1]);
    await user.keyboard("{Meta>}");
    await user.click(rowEls()[3]);
    await user.keyboard("{/Meta}");
    expect(selectedRows()).toHaveLength(2);
    await user.keyboard("{Shift>}");
    await user.click(rowEls()[6]);
    await user.keyboard("{/Shift}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48003", "48004", "48005", "48006"]);
  });

  it("shows loading and empty states instead of rows", () => {
    const { rerender } = render(<Harness data={[]} loading />);
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();
    rerender(<Harness data={[]} empty={<p>No dead letters</p>} />);
    expect(screen.getByText("No dead letters")).toBeInTheDocument();
  });

  it("selects all with mod+A", async () => {
    render(<Harness data={rows(12)} />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("j");
    await userEvent.keyboard("{Control>}a{/Control}");
    expect(selectedRows()).toHaveLength(12);
  });

  it("jumps with Home and End and moves up with k", async () => {
    render(<Harness data={rows(12)} />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("{End}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48011"]);
    await userEvent.keyboard("k");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48010"]);
    await userEvent.keyboard("{Home}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48000"]);
  });

  it("extends upwards with Shift+ArrowUp", async () => {
    render(<Harness data={rows(12)} />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("jjjjj"); // focus 48004
    await userEvent.keyboard("{Shift>}{ArrowUp}{ArrowUp}{/Shift}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48002", "48003", "48004"]);
  });

  it("points aria-activedescendant at the focused row", async () => {
    render(<Harness data={rows(12)} />);
    const grid = screen.getByRole("grid");
    expect(grid).not.toHaveAttribute("aria-activedescendant");
    grid.focus();
    await userEvent.keyboard("jj");
    const id = grid.getAttribute("aria-activedescendant");
    expect(id).toBeTruthy();
    const el = document.getElementById(id!);
    expect(el).not.toBeNull();
    expect(el).toHaveAttribute("data-row-id", "48001");
  });

  it("shows the error instead of rows", () => {
    render(<Harness error={<p>Can&apos;t peek messages</p>} />);
    expect(screen.getByText("Can't peek messages")).toBeInTheDocument();
    expect(screen.queryByText("Subject 0")).not.toBeInTheDocument();
  });

  it("shapes loading rows to the grid's columns", () => {
    render(<Harness data={[]} loading />);
    const row = screen.getByRole("status").querySelector("[data-skeleton-row]") as HTMLElement;
    expect(row.style.gridTemplateColumns).toBe("64px minmax(140px, 1fr) 60px");
    expect(row.children).toHaveLength(3);
    expect(row.style.height).toBe("22px");
  });

  it("re-measures when the row height changes", () => {
    const { rerender } = render(<Harness data={rows(10)} rowHeight={22} />);
    const third = () => screen.getAllByRole("row")[3] as HTMLElement;
    expect(third().style.height).toBe("22px");
    expect(third().style.transform).toBe("translateY(44px)");
    act(() => { rerender(<Harness data={rows(10)} rowHeight={30} />); });
    expect(third().style.height).toBe("30px");
    expect(third().style.transform).toBe("translateY(60px)");
    const body = third().parentElement as HTMLElement;
    expect(body.style.height).toBe("300px");
  });

  it("outlines the focus row only when it is not the sole selection", async () => {
    const user = userEvent.setup();
    render(<Harness data={rows(12)} />);
    const rowEls = () => screen.getAllByRole("row").slice(1);
    await user.click(rowEls()[1]);
    expect(rowEls()[1].className).not.toContain("outline-muted");
    await user.keyboard("{Meta>}");
    await user.click(rowEls()[3]);
    await user.keyboard("{/Meta}");
    expect(rowEls()[3].className).toContain("outline-muted");
  });
});
