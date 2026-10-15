import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";
import { cn } from "../lib/cn";

export const buttonVariants = cva(
  "inline-flex select-none items-center justify-center gap-1.5 whitespace-nowrap rounded-ctl text-12 font-normal disabled:pointer-events-none disabled:opacity-45 [&_svg]:size-3.5 [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "border border-line bg-field text-fg hover:bg-hover active:bg-selection",
        quiet: "text-fg hover:bg-hover active:bg-selection",
        primary: "border border-fg bg-field font-semibold text-fg hover:bg-hover active:bg-selection focus-visible:outline-offset-1",
        danger: "border border-line bg-field text-fg hover:bg-hover active:bg-selection",
      },
      size: { sm: "h-[22px] px-1.5", md: "h-6 px-2", lg: "h-7 px-3 text-13" },
    },
    defaultVariants: { variant: "default", size: "md" },
  },
);

export function Button({
  className,
  variant,
  size,
  type = "button",
  ...props
}: ComponentProps<"button"> & VariantProps<typeof buttonVariants>) {
  return <button type={type} className={cn(buttonVariants({ variant, size }), className)} {...props} />;
}
