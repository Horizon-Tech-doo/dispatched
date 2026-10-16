import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ColumnDef } from "@tanstack/react-table";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { DataGrid, rowNumberWidth, type DataGridProps, type GridSort } from "./DataGrid";

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

  it("marks the keyboard focus on the active row, and on the container only when no row is active", async () => {
    const user = userEvent.setup();
    render(<Harness data={rows(12)} />);
    const ring = () => document.querySelector<HTMLElement>("[data-focus-ring]")!;
    // Nothing active: the container carries the focus ring.
    expect(ring().dataset.focusRing).toBe("container");
    expect(ring().className).toContain("peer-focus-visible:outline-fg");
    const rowEls = () => screen.getAllByRole("row").slice(1);
    await user.click(rowEls()[2]);
    // An active row: its inset outline is the mark and the container draws none.
    expect(ring().dataset.focusRing).toBe("row");
    expect(ring().className).not.toContain("peer-focus-visible");
    expect(rowEls()[2].className).toContain("group-focus-visible/grid:outline-fg");
    expect(rowEls()[1].className).not.toContain("group-focus-visible/grid:outline-fg");
  });

  it("shares the panel's width between the columns instead of running past it", () => {
    render(<Harness />);
    const header = screen.getAllByRole("row")[0];
    expect(header.parentElement!.className).toContain("w-full");
    expect(header.parentElement!.className).not.toContain("w-max");
  });
});

const sortable: ColumnDef<Msg, unknown>[] = [
  { id: "seq", header: "Seq", accessorFn: (r) => r.seq, meta: { mono: true, width: "64px" } },
  { id: "subject", header: "Subject", accessorFn: (r) => r.subject },
];

function Sorted(props: Partial<DataGridProps<Msg>> & { initial?: GridSort | null }) {
  const { initial, ...rest } = props;
  const [sorting, setSorting] = useState<GridSort | null>(initial ?? null);
  const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: null });
  return (
    <DataGrid<Msg>
      label="Messages"
      columns={sortable}
      rows={rows(5)}
      getRowId={(r) => String(r.seq)}
      selectedIds={sel.ids}
      focusId={sel.focus}
      onSelectionChange={(ids, focus) => setSel({ ids, focus })}
      rowHeight={22}
      initialRect={{ width: 800, height: 600 }}
      sorting={sorting}
      onSortingChange={setSorting}
      {...rest}
    />
  );
}

const seqColumn = () => screen.getAllByRole("row").slice(1).map((r) => within(r).getAllByRole("gridcell")[0].textContent);
const sortButton = () => within(screen.getByRole("columnheader", { name: /Seq/ })).getByRole("button");

describe("DataGrid sorting and columns", () => {
  it("cycles a header through ascending, descending and none", async () => {
    render(<Sorted />);
    const header = screen.getByRole("columnheader", { name: /Seq/ });
    await userEvent.click(sortButton());
    expect(header).toHaveAttribute("aria-sort", "ascending");
    expect(seqColumn()[0]).toBe("48000");
    await userEvent.click(sortButton());
    expect(header).toHaveAttribute("aria-sort", "descending");
    expect(seqColumn()[0]).toBe("48004");
    await userEvent.click(sortButton());
    expect(header).not.toHaveAttribute("aria-sort");
    expect(seqColumn()[0]).toBe("48000");
  });

  it("reports sort changes without owning the state", async () => {
    const onSortingChange = vi.fn();
    render(<Sorted sorting={null} onSortingChange={onSortingChange} />);
    await userEvent.click(sortButton());
    expect(onSortingChange).toHaveBeenCalledWith({ id: "seq", desc: false });
  });

  it("offers no sort buttons without a sort handler or when a column opts out", () => {
    const { rerender } = render(<Sorted onSortingChange={undefined} />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
    const optOut: ColumnDef<Msg, unknown>[] = [{ ...sortable[0], enableSorting: false }, sortable[1]];
    rerender(<Sorted columns={optOut} />);
    expect(within(screen.getByRole("columnheader", { name: /Seq/ })).queryByRole("button")).not.toBeInTheDocument();
    expect(within(screen.getByRole("columnheader", { name: /Subject/ })).getByRole("button")).toBeInTheDocument();
  });

  it("numbers rows in display order", () => {
    render(<Sorted rowNumbers initial={{ id: "seq", desc: true }} />);
    expect(screen.getAllByRole("columnheader")[0]).toHaveTextContent("#");
    const first = screen.getAllByRole("row")[1];
    expect(within(first).getAllByRole("gridcell")[0]).toHaveTextContent("1");
    expect(within(first).getAllByRole("gridcell")[1]).toHaveTextContent("48004");
  });

  it("keeps a shift range anchored to the row after re-sorting", async () => {
    render(<Sorted />);
    const grid = screen.getByRole("grid");
    await userEvent.click(screen.getAllByRole("row")[1]); // the smallest seq, 48000
    await userEvent.click(sortButton());
    await userEvent.click(sortButton()); // descending: 48000 is now the last row
    // Shift+K extends upwards from the anchor; Shift+J would clamp at the end and select one row.
    await userEvent.keyboard("{Shift>}K{/Shift}");
    const selected = [...grid.querySelectorAll('[aria-selected="true"]')].map((r) => within(r as HTMLElement).getAllByRole("gridcell")[0].textContent);
    expect(selected.sort()).toEqual(["48000", "48001"]);
  });

  it("extends with Shift+J like Shift+ArrowDown", async () => {
    render(<Sorted />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("j{Shift>}JJ{/Shift}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48000", "48001", "48002"]);
  });

  it("moves a page with PageDown and PageUp", async () => {
    render(<Harness data={rows(200)} />);
    const grid = screen.getByRole("grid");
    grid.focus();
    await userEvent.keyboard("{ArrowDown}{PageDown}");
    // jsdom has no layout, so the page falls back to ten rows minus one.
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48009"]);
    expect(grid.getAttribute("aria-activedescendant")).toMatch(/-48009$/);
    await userEvent.keyboard("{PageUp}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48000"]);
  });

  it("does not hijack Enter on a header button", async () => {
    const onActivate = vi.fn();
    render(<Sorted onActivate={onActivate} />);
    await userEvent.click(screen.getAllByRole("row")[1]);
    sortButton().focus();
    await userEvent.keyboard("{Enter}");
    expect(onActivate).not.toHaveBeenCalled();
    expect(screen.getByRole("columnheader", { name: /Seq/ })).toHaveAttribute("aria-sort", "ascending");
  });

  it("marks itself busy while loading", () => {
    const { rerender } = render(<Harness data={rows(2000)} />);
    expect(screen.getByRole("grid")).not.toHaveAttribute("aria-busy");
    rerender(<Harness data={[]} loading />);
    expect(screen.getByRole("grid")).toHaveAttribute("aria-busy", "true");
  });

  it("never points aria-activedescendant at a row that is not rendered", async () => {
    render(<Harness data={rows(2000)} />);
    const grid = screen.getByRole("grid");
    grid.focus();
    for (const key of ["{End}", "{Home}", "{PageDown}", "{End}"]) {
      await userEvent.keyboard(key);
      const id = grid.getAttribute("aria-activedescendant");
      if (id) expect(document.getElementById(id)).not.toBeNull();
    }
    // jsdom does not scroll, so row 49999 is outside the rendered window after End.
    expect(document.querySelector('[data-row-id="49999"]')).toBeNull();
    expect(grid).not.toHaveAttribute("aria-activedescendant");
  });

  it("puts loading, empty and error bodies in a row and gridcell", () => {
    const { rerender } = render(<Harness data={[]} loading />);
    const cellOf = (el: HTMLElement) => el.closest('[role="gridcell"]') as HTMLElement;
    expect(cellOf(screen.getByRole("status")).parentElement).toHaveAttribute("role", "row");
    expect(cellOf(screen.getByRole("status"))).toHaveAttribute("aria-colspan", "3");
    rerender(<Harness data={[]} empty={<p>No dead letters</p>} />);
    expect(cellOf(screen.getByText("No dead letters")).parentElement).toHaveAttribute("role", "row");
    rerender(<Harness data={[]} error={<p>Can&apos;t peek</p>} />);
    expect(cellOf(screen.getByText("Can't peek"))).toHaveAttribute("aria-colspan", "3");
  });

  it("anchors a range at the preset focus row when nothing was selected yet", async () => {
    function Preset() {
      const [sel, setSel] = useState<{ ids: Set<string>; focus: string | null }>({ ids: new Set(), focus: "48000" });
      return (
        <DataGrid<Msg>
          label="Messages"
          columns={sortable}
          rows={rows(10)}
          getRowId={(r) => String(r.seq)}
          selectedIds={sel.ids}
          focusId={sel.focus}
          onSelectionChange={(ids, focus) => setSel({ ids, focus })}
          rowHeight={22}
          initialRect={{ width: 800, height: 600 }}
        />
      );
    }
    render(<Preset />);
    screen.getByRole("grid").focus();
    await userEvent.keyboard("{Shift>}{ArrowDown}{ArrowDown}{ArrowDown}{/Shift}");
    expect(selectedRows().map((r) => r.getAttribute("data-row-id"))).toEqual(["48000", "48001", "48002", "48003"]);
  });

  it("returns focus to the grid when a navigation key is pressed on a header button", async () => {
    render(<Sorted />);
    const grid = screen.getByRole("grid");
    sortButton().focus();
    await userEvent.keyboard("j");
    expect(grid).toHaveFocus();
    expect(grid.getAttribute("aria-activedescendant")).toMatch(/-48000$/);
    sortButton().focus();
    await userEvent.keyboard(" ");
    expect(sortButton()).toHaveFocus();
  });

  it("renders a bounded window of a sorted 10,000-row list", () => {
    render(<Sorted rows={rows(10_000)} initial={{ id: "seq", desc: true }} />);
    const rendered = screen.getAllByRole("row").length;
    expect(rendered).toBeLessThan(120);
    expect(seqColumn()[0]).toBe("57999");
  });

  it("widens the row-number gutter past five digits only", () => {
    expect(rowNumberWidth(20)).toBe("52px");
    expect(rowNumberWidth(99_999)).toBe("52px");
    expect(rowNumberWidth(100_000)).toBe("60px");
    expect(rowNumberWidth(1_000_000)).toBe("67px");
    render(<Sorted rowNumbers />);
    expect(screen.getAllByRole("row")[0].style.gridTemplateColumns.startsWith("52px ")).toBe(true);
  });

  it("labels the row-number column", () => {
    render(<Sorted rowNumbers />);
    expect(screen.getByRole("columnheader", { name: "Row number" })).toHaveTextContent("#");
  });
});
