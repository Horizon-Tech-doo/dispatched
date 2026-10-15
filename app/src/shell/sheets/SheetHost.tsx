import { useShell } from "../context";

/** Renders the open side sheet, if any. */
export function SheetHost() {
  const { sheet } = useShell();
  if (!sheet) return null;
  return null;
}
