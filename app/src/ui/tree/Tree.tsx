import { useVirtualizer } from "@tanstack/react-virtual";
import { ChevronRight } from "lucide-react";
import { useCallback, useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { cn } from "../lib/cn";
import { observeRectWithFloor } from "../lib/virtual";

export interface TreeNode {
  id: string;
  parentId: string | null;
  depth: number;
  label: string;
  expandable: boolean;
  expanded: boolean;
}

export interface TreeRowState {
  selected: boolean;
}

export interface TreeProps<N extends TreeNode> {
  label: string;
  nodes: N[];
  selectedId: string | null;
  onSelect(id: string): void;
  onActivate(id: string): void;
  onToggle(id: string, expanded: boolean): void;
  renderRow(node: N, state: TreeRowState): ReactNode;
  rowHeight: number;
  className?: string;
  initialRect?: { width: number; height: number };
}

const INDENT = 14;
const TYPEAHEAD_MS = 500;

export function Tree<N extends TreeNode>(props: TreeProps<N>) {
  const { label, nodes, selectedId, onSelect, onActivate, onToggle, renderRow, rowHeight, className, initialRect } = props;
  const domId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const typeahead = useRef({ text: "", at: 0 });

  const virtualizer = useVirtualizer({
    count: nodes.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    initialRect,
    observeElementRect: observeRectWithFloor(initialRect),
  });

  useEffect(() => virtualizer.measure(), [rowHeight, virtualizer]);

  const index = nodes.findIndex((n) => n.id === selectedId);

  const select = useCallback(
    (i: number) => {
      const n = nodes[Math.max(0, Math.min(nodes.length - 1, i))];
      if (!n) return;
      onSelect(n.id);
      virtualizer.scrollToIndex(nodes.indexOf(n), { align: "auto" });
    },
    [nodes, onSelect, virtualizer],
  );

  function onKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const cur = nodes[index];
    // Mid-word, j/k are letters: only a first key navigates.
    const typing = typeahead.current.text !== "" && Date.now() - typeahead.current.at < TYPEAHEAD_MS;
    switch (e.key) {
      case "ArrowDown":
      case "j":
        if (typing && e.key.length === 1) break;
        e.preventDefault();
        select(index < 0 ? 0 : index + 1);
        return;
      case "ArrowUp":
      case "k":
        if (typing && e.key.length === 1) break;
        e.preventDefault();
        select(index < 0 ? 0 : index - 1);
        return;
      case "Home":
        e.preventDefault();
        select(0);
        return;
      case "End":
        e.preventDefault();
        select(nodes.length - 1);
        return;
      case "ArrowRight":
        if (!cur) return;
        e.preventDefault();
        if (cur.expandable && !cur.expanded) onToggle(cur.id, true);
        else if (cur.expandable && cur.expanded && nodes[index + 1]?.parentId === cur.id) select(index + 1);
        return;
      case "ArrowLeft":
        if (!cur) return;
        e.preventDefault();
        if (cur.expandable && cur.expanded) onToggle(cur.id, false);
        else if (cur.parentId) select(nodes.findIndex((n) => n.id === cur.parentId));
        return;
      case "Enter":
        if (cur) {
          e.preventDefault();
          onActivate(cur.id);
        }
        return;
    }
    if (e.key.length === 1 && e.key !== " ") {
      const now = Date.now();
      const t = typeahead.current;
      t.text = now - t.at < TYPEAHEAD_MS ? t.text + e.key.toLowerCase() : e.key.toLowerCase();
      t.at = now;
      // Start at the current row when extending a prefix, after it for a new one.
      const start = t.text.length > 1 ? Math.max(index, 0) : index + 1;
      for (let k = 0; k < nodes.length; k++) {
        const i = (start + k) % nodes.length;
        if (nodes[i].label.toLowerCase().startsWith(t.text)) {
          select(i);
          break;
        }
      }
    }
  }

  const rowId = (id: string) => `${domId}-${id}`;

  return (
    <div
      ref={scrollRef}
      role="tree"
      aria-label={label}
      tabIndex={0}
      aria-activedescendant={selectedId && index >= 0 ? rowId(selectedId) : undefined}
      onKeyDown={onKeyDown}
      className={cn("relative h-full overflow-y-auto focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg", className)}
    >
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((v) => {
          const n = nodes[v.index];
          const selected = n.id === selectedId;
          return (
            <div
              key={n.id}
              id={rowId(n.id)}
              data-node-id={n.id}
              role="treeitem"
              aria-level={n.depth + 1}
              aria-expanded={n.expandable ? n.expanded : undefined}
              aria-selected={selected}
              aria-label={n.label}
              onClick={() => onSelect(n.id)}
              onDoubleClick={() => onActivate(n.id)}
              className={cn("absolute inset-x-0 flex items-center pr-2.5", selected ? "bg-selection" : "hover:bg-hover")}
              style={{ height: rowHeight, transform: `translateY(${v.start}px)`, paddingLeft: 6 + n.depth * INDENT }}
            >
              <span
                className="grid w-3.5 shrink-0 place-items-center text-faint"
                onDoubleClick={(e) => e.stopPropagation()}
                onClick={(e) => {
                  if (!n.expandable) return;
                  e.stopPropagation();
                  onToggle(n.id, !n.expanded);
                }}
              >
                {n.expandable && <ChevronRight aria-hidden="true" className={cn("size-3", n.expanded && "rotate-90")} />}
              </span>
              <div className="flex min-w-0 flex-1 items-center gap-1.5 pl-1">{renderRow(n, { selected })}</div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
