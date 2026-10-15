import { Command as Cmdk } from "cmdk";
import { Search } from "lucide-react";
import { Dialog } from "radix-ui";
import type { ComponentProps, ReactNode } from "react";
import { cn } from "../lib/cn";

/** The ⌘K palette: centred near the top, no dimming, 560px wide. */
export function CommandPaletteDialog({ open, onOpenChange, children, label = "Command palette" }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: ReactNode;
  label?: string;
}) {
  return (
    <Dialog.Root open={open} onOpenChange={onOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-40 bg-transparent" />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[72px] z-50 w-[560px] max-w-[calc(100vw-32px)] -translate-x-1/2 overflow-hidden rounded-ctl border border-line bg-side text-fg shadow-float outline-none"
        >
          <Dialog.Title className="sr-only">{label}</Dialog.Title>
          <Cmdk label={label} loop className="flex flex-col">{children}</Cmdk>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

export function CommandInput({ className, ...props }: ComponentProps<typeof Cmdk.Input>) {
  return (
    <div className="flex h-9 items-center gap-2 border-b border-line px-3 focus-within:outline focus-within:outline-1 focus-within:-outline-offset-1 focus-within:outline-fg">
      <Search aria-hidden="true" className="size-3.5 text-icon" />
      <Cmdk.Input className={cn("h-full flex-1 bg-transparent text-13 text-fg outline-none placeholder:text-faint", className)} {...props} />
    </div>
  );
}

export function CommandList({ className, ...props }: ComponentProps<typeof Cmdk.List>) {
  return <Cmdk.List className={cn("max-h-[360px] overflow-y-auto py-1", className)} {...props} />;
}

export function CommandEmpty(props: ComponentProps<typeof Cmdk.Empty>) {
  return <Cmdk.Empty className="px-3 py-3 text-12 text-muted" {...props} />;
}

export function CommandGroup({ className, ...props }: ComponentProps<typeof Cmdk.Group>) {
  return (
    <Cmdk.Group
      className={cn("[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pb-0.5 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:text-11 [&_[cmdk-group-heading]]:text-muted", className)}
      {...props}
    />
  );
}

export function CommandItem({ className, ...props }: ComponentProps<typeof Cmdk.Item>) {
  return (
    <Cmdk.Item
      className={cn("flex h-row cursor-default items-center gap-2 px-3 text-12 outline-none data-[selected=true]:bg-selection [&_svg]:size-3.5 [&_svg]:text-icon", className)}
      {...props}
    />
  );
}

export function CommandShortcut({ className, ...props }: ComponentProps<"span">) {
  return <span className={cn("ml-auto font-mono text-11 text-muted", className)} {...props} />;
}
