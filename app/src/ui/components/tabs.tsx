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

  // Keep the active tab visible when many tabs overflow the strip. Only the list scrolls: `scrollIntoView`
  // would also scroll every scrollable ancestor, the window included.
  useEffect(() => {
    const list = listRef.current;
    if (!list || typeof MutationObserver === "undefined") return;
    const reveal = () => {
      const tab = list.querySelector<HTMLElement>('[role="tab"][data-state="active"]');
      if (!tab) return;
      const l = list.getBoundingClientRect();
      const t = tab.getBoundingClientRect();
      if (t.left < l.left) list.scrollLeft -= l.left - t.left;
      else if (t.right > l.right) list.scrollLeft += t.right - l.right;
    };
    reveal();
    const observer = new MutationObserver(reveal);
    observer.observe(list, { attributes: true, attributeFilter: ["data-state"], subtree: true });
    return () => observer.disconnect();
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
