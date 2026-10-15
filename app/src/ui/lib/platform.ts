export const isMac =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

const MAC: Record<string, string> = { mod: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃" };
const OTHER: Record<string, string> = { mod: "Ctrl", shift: "Shift", alt: "Alt", ctrl: "Ctrl" };
const MAC_ORDER = ["ctrl", "alt", "shift", "mod"];

/** `"mod+shift+p"` → `"⇧⌘P"` on macOS, `"Ctrl+Shift+P"` elsewhere. */
export function shortcutLabel(spec: string, mac: boolean = isMac): string {
  const parts = spec.toLowerCase().split("+");
  const key = parts.pop()!;
  const keyLabel = key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1);
  if (mac) {
    const mods = MAC_ORDER.filter((m) => parts.includes(m)).map((m) => MAC[m]);
    return mods.join("") + keyLabel;
  }
  return [...parts.map((m) => OTHER[m] ?? m), keyLabel].join("+");
}
