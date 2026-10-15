import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { Tree, type TreeNode } from "./Tree";

const RECT = { width: 300, height: 220 };

function node(id: string, parentId: string | null, depth: number, expandable = false, expanded = false): TreeNode {
  return { id, parentId, depth, label: id, expandable, expanded };
}

const NODES: TreeNode[] = [
  node("queues", null, 0, true, true),
  node("invoice-generate", "queues", 1),
  node("label-print", "queues", 1),
  node("topics", null, 0, true, true),
  node("shipment-events", "topics", 1, true, false),
  node("tracking-sync", "topics", 1),
];

function Harness(props: { nodes?: TreeNode[]; onToggle?: (id: string, e: boolean) => void; onActivate?: (id: string) => void }) {
  const [sel, setSel] = useState<string | null>("queues");
  return (
    <Tree
      label="Namespace"
      nodes={props.nodes ?? NODES}
      selectedId={sel}
      onSelect={setSel}
      onActivate={props.onActivate ?? (() => {})}
      onToggle={props.onToggle ?? (() => {})}
      renderRow={(n) => <span>{n.label}</span>}
      rowHeight={22}
      initialRect={RECT}
    />
  );
}

const selected = () => screen.getAllByRole("treeitem").find((r) => r.getAttribute("aria-selected") === "true");

describe("Tree", () => {
  it("renders_only_visible_rows_of_5000", () => {
    const many = Array.from({ length: 5000 }, (_, i) => node(`q-${i}`, null, 0));
    render(<Harness nodes={many} />);
    expect(screen.getAllByRole("treeitem").length).toBeLessThan(60);
  });

  it("exposes tree semantics", () => {
    render(<Harness />);
    const tree = screen.getByRole("tree", { name: "Namespace" });
    expect(tree).toHaveAttribute("tabindex", "0");
    const topics = within(tree).getByRole("treeitem", { name: "topics" });
    expect(topics).toHaveAttribute("aria-level", "1");
    expect(topics).toHaveAttribute("aria-expanded", "true");
    expect(within(tree).getByRole("treeitem", { name: "tracking-sync" })).toHaveAttribute("aria-level", "2");
    expect(within(tree).getByRole("treeitem", { name: "tracking-sync" })).not.toHaveAttribute("aria-expanded");
  });

  it("moves with arrows and j/k, Home and End", async () => {
    render(<Harness />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{ArrowDown}j");
    expect(selected()).toHaveAccessibleName("label-print");
    await userEvent.keyboard("k");
    expect(selected()).toHaveAccessibleName("invoice-generate");
    await userEvent.keyboard("{End}");
    expect(selected()).toHaveAccessibleName("tracking-sync");
    await userEvent.keyboard("{Home}");
    expect(selected()).toHaveAccessibleName("queues");
  });

  it("expands, collapses and moves to the parent", async () => {
    const onToggle = vi.fn();
    render(<Harness onToggle={onToggle} />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{End}{ArrowUp}{ArrowRight}");
    expect(onToggle).toHaveBeenCalledWith("shipment-events", true);
    await userEvent.keyboard("{Home}{ArrowLeft}");
    expect(onToggle).toHaveBeenCalledWith("queues", false);
    await userEvent.keyboard("{ArrowDown}{ArrowLeft}");
    expect(selected()).toHaveAccessibleName("queues");
  });

  it("activates with Enter and double-click", async () => {
    const onActivate = vi.fn();
    render(<Harness onActivate={onActivate} />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("{End}{Enter}");
    expect(onActivate).toHaveBeenCalledWith("tracking-sync");
    await userEvent.dblClick(screen.getByRole("treeitem", { name: "label-print" }));
    expect(onActivate).toHaveBeenCalledWith("label-print");
  });

  it("type-ahead jumps to the next matching label", async () => {
    render(<Harness />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("tr");
    expect(selected()).toHaveAccessibleName("tracking-sync");
  });

  it("treats j and k as letters while a type-ahead is in progress", async () => {
    render(<Harness />);
    screen.getByRole("tree").focus();
    await userEvent.keyboard("tracking");
    expect(selected()).toHaveAccessibleName("tracking-sync");
  });

  it("clicking a chevron toggles without selecting or activating", async () => {
    const onToggle = vi.fn();
    const onActivate = vi.fn();
    render(<Harness onToggle={onToggle} onActivate={onActivate} />);
    const chevron = screen.getByRole("treeitem", { name: "shipment-events" }).firstElementChild as HTMLElement;
    await userEvent.dblClick(chevron);
    expect(onToggle).toHaveBeenCalledWith("shipment-events", true);
    expect(onActivate).not.toHaveBeenCalled();
    expect(selected()).toHaveAccessibleName("queues");
  });
});
