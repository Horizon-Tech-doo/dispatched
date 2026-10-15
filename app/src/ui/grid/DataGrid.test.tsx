import { render, screen, within } from "@testing-library/react";
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

function Harness({ data = rows(20), onActivate = () => {}, ...rest }: { data?: Msg[]; onActivate?: (id: string) => void; loading?: boolean; empty?: React.ReactNode }) {
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
});
