import { Tabs as TabsPrimitive } from "radix-ui";
import { createContext, useContext, useEffect, useRef, type ComponentProps, type WheelEvent } from "react";
import { cn } from "../lib/cn";

type Variant = "strip" | "underline";
const VariantContext = createContext<Variant>("underline");

export const Tabs = TabsPrimitive.Root;

export function TabsList({
  variant = "underline",
  className,
  ref,
  onWheel,
  ...props
}: ComponentProps<typeof TabsPrimitive.List> & { variant?: Variant }) {
  const listRef = useRef<HTMLDivElement | null>(null);

  // When the tabs do not fit, the list scrolls sideways and the active tab is kept in view. Only the list scrolls:
  // `scrollIntoView` would also scroll every scrollable ancestor, the window included. `data-overflow-start` and
  // `data-overflow-end` mark which edges have more tabs beyond them; they fade in a gradient (the classes below).
  useEffect(() => {
    const list = listRef.current;
    if (!list || typeof MutationObserver === "undefined") return;
    // Set once the person scrolls the strip themselves: tabs resizing afterwards (a count arriving) must not undo it.
    let userScrolled = false;
    const edges = () => {
      const more = list.scrollWidth - list.clientWidth > 1;
      const start = more && list.scrollLeft > 1;
      const end = more && list.scrollLeft + list.clientWidth < list.scrollWidth - 1;
      list.toggleAttribute("data-overflow-start", start);
      list.toggleAttribute("data-overflow-end", end);
    };
    const reveal = () => {
      const tab = list.querySelector<HTMLElement>('[role="tab"][data-state="active"]');
      if (tab) {
        const l = list.getBoundingClientRect();
        const t = tab.getBoundingClientRect();
        if (t.left < l.left) list.scrollLeft -= l.left - t.left;
        else if (t.right > l.right) list.scrollLeft += t.right - l.right;
        // The first and last tab reveal the strip's own padding too, so no fade claims there are more tabs.
        const tabs = list.querySelectorAll('[role="tab"]');
        if (tab === tabs[0]) list.scrollLeft = 0;
        else if (tab === tabs[tabs.length - 1]) list.scrollLeft = Math.max(list.scrollLeft, list.scrollWidth - list.clientWidth);
      }
      edges();
    };
    const onActivated = () => {
      userScrolled = false;
      reveal();
    };
    const onUser = () => (userScrolled = true);
    reveal();
    const states = new MutationObserver((records) => {
      if (records.some((r) => r.type === "attributes")) onActivated();
      else edges();
    });
    states.observe(list, { attributes: true, attributeFilter: ["data-state"], childList: true, subtree: true });
    // The strip or a tab changed size (the window, a count arriving): keep the active tab in view unless the person scrolled.
    const sizes = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => (userScrolled ? edges() : reveal()));
    sizes?.observe(list);
    for (const child of Array.from(list.children)) sizes?.observe(child);
    list.addEventListener("scroll", edges, { passive: true });
    list.addEventListener("wheel", onUser, { passive: true });
    list.addEventListener("pointerdown", onUser, { passive: true });
    return () => {
      states.disconnect();
      sizes?.disconnect();
      list.removeEventListener("scroll", edges);
      list.removeEventListener("wheel", onUser);
      list.removeEventListener("pointerdown", onUser);
    };
  }, []);

  const handleWheel = (event: WheelEvent<HTMLDivElement>) => {
    onWheel?.(event);
    // A mouse wheel only reports vertical movement; let it scroll the strip sideways.
    if (event.defaultPrevented || event.deltaX !== 0 || event.deltaY === 0) return;
    event.currentTarget.scrollLeft += event.deltaY;
  };

  return (
    <VariantContext.Provider value={variant}>
      <TabsPrimitive.List
        ref={(node) => {
          listRef.current = node;
          if (typeof ref === "function") ref(node);
          else if (ref) ref.current = node;
        }}
        onWheel={handleWheel}
        className={cn(
          "flex min-w-0 items-stretch overflow-x-auto [scrollbar-width:none]",
          // A soft edge where more tabs are hidden: zero-width sticky ends that paint a short gradient over the tabs.
          "[--tabs-fade:var(--c-content)] before:pointer-events-none before:sticky before:left-0 before:z-10 before:-mr-7 before:w-7 before:shrink-0 before:self-stretch before:bg-linear-to-r before:from-(--tabs-fade) before:to-transparent before:opacity-0 before:content-['']",
          "after:pointer-events-none after:sticky after:right-0 after:z-10 after:-ml-7 after:w-7 after:shrink-0 after:self-stretch after:bg-linear-to-l after:from-(--tabs-fade) after:to-transparent after:opacity-0 after:content-['']",
          "data-[overflow-start]:before:opacity-100 data-[overflow-end]:after:opacity-100",
          variant === "strip" && "[--tabs-fade:var(--c-chrome)]",
          variant === "strip" ? "h-[30px] bg-chrome" : "h-7 gap-0.5 px-2.5",
          className,
        )}
        {...props}
      />
    </VariantContext.Provider>
  );
}

export function TabsTrigger({ className, onMouseDown, onKeyDown, onBlur, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  const variant = useContext(VariantContext);
  return (
    <TabsPrimitive.Trigger
      // Radix focuses a tab from script when it is pressed, and browsers then match :focus-visible, so a click
      // would draw the keyboard ring. A tab pressed with the pointer is marked and shows only its marker; the
      // mark goes with the next key or when focus leaves.
      onMouseDown={(e) => {
        e.currentTarget.dataset.pointer = "";
        onMouseDown?.(e);
      }}
      onKeyDown={(e) => {
        delete e.currentTarget.dataset.pointer;
        onKeyDown?.(e);
      }}
      onBlur={(e) => {
        delete e.currentTarget.dataset.pointer;
        onBlur?.(e);
      }}
      className={cn(
        "flex shrink-0 items-center gap-1.5 whitespace-nowrap text-12 text-muted hover:text-fg data-[state=active]:text-fg",
        variant === "strip"
          ? "border-r border-t-2 border-line border-t-transparent px-3 data-[state=active]:border-t-fg data-[state=active]:bg-content"
          : "border-b-2 border-b-transparent px-2 data-[state=active]:border-b-fg",
        // Inset by 2px: inside the marker's edge, so a focused tab never looks like a second selected one.
        "focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-2 focus-visible:outline-fg",
        "data-[pointer]:focus-visible:outline-0",
        className,
      )}
      {...props}
    />
  );
}

export function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content className={cn("min-h-0 flex-1 focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg", className)} {...props} />;
}
