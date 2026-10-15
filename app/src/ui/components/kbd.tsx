import { cn } from "../lib/cn";
import { shortcutLabel } from "../lib/platform";

export function Kbd({ shortcut, className }: { shortcut: string; className?: string }) {
  return <kbd className={cn("font-mono text-11 text-muted", className)}>{shortcutLabel(shortcut)}</kbd>;
}
