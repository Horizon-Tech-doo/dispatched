import { Tabs as TabsPrimitive } from "radix-ui";
import { createContext, useContext, type ComponentProps } from "react";
import { cn } from "../lib/cn";

type Variant = "strip" | "underline";
const VariantContext = createContext<Variant>("underline");

export const Tabs = TabsPrimitive.Root;

export function TabsList({ variant = "underline", className, ...props }: ComponentProps<typeof TabsPrimitive.List> & { variant?: Variant }) {
  return (
    <VariantContext.Provider value={variant}>
      <TabsPrimitive.List
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

export function TabsTrigger({ className, ...props }: ComponentProps<typeof TabsPrimitive.Trigger>) {
  const variant = useContext(VariantContext);
  return (
    <TabsPrimitive.Trigger
      className={cn(
        "group flex shrink-0 items-center gap-1.5 whitespace-nowrap text-12 text-muted outline-none hover:text-fg data-[state=active]:text-fg",
        variant === "strip"
          ? "border-r border-line px-3 data-[state=active]:bg-content data-[state=active]:shadow-[inset_0_2px_0_var(--c-fg)]"
          : "px-2 data-[state=active]:shadow-[inset_0_-2px_0_var(--c-fg)]",
        "focus-visible:outline focus-visible:outline-1 focus-visible:-outline-offset-1 focus-visible:outline-fg",
        className,
      )}
      {...props}
    />
  );
}

export function TabsContent({ className, ...props }: ComponentProps<typeof TabsPrimitive.Content>) {
  return <TabsPrimitive.Content className={cn("min-h-0 flex-1 outline-none", className)} {...props} />;
}
