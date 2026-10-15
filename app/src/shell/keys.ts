import { useEffect, useRef } from "react";
import { isMac } from "@/ui";

/** `"mod+k"`, `"mod+shift+p"`, `"f5"`, `"mod+,"`: mod is Cmd on macOS and Ctrl elsewhere. Modifiers must match exactly. */
export function matches(e: KeyboardEvent, combo: string, mac: boolean = isMac): boolean {
  const parts = combo.toLowerCase().split("+");
  // A combo that ends in "+" is the plus key itself: "mod++" splits to ["mod", "", ""].
  const key = parts.pop() || "+";
  const want = { mod: parts.includes("mod"), shift: parts.includes("shift"), alt: parts.includes("alt") };
  const mod = mac ? e.metaKey : e.ctrlKey;
  const otherMod = mac ? e.ctrlKey : e.metaKey;
  return e.key.toLowerCase() === key && mod === want.mod && e.shiftKey === want.shift && e.altKey === want.alt && !otherMod;
}

export interface Binding {
  combo: string;
  run(e: KeyboardEvent): void;
  /** Fire while a text field has focus. Without it the key is left to the field. */
  inInputs?: boolean;
}

const typing = (t: EventTarget | null) =>
  t instanceof HTMLElement && (t.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(t.tagName));

export function useHotkeys(bindings: Binding[]): void {
  const ref = useRef(bindings);
  ref.current = bindings;
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      for (const b of ref.current) {
        if (!matches(e, b.combo)) continue;
        if (typing(e.target) && !b.inInputs) continue;
        e.preventDefault();
        b.run(e);
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}
