export type ThemeName = "dark" | "light";
export type Role =
  | "chrome" | "side" | "content" | "field" | "line"
  | "fg" | "muted" | "faint" | "icon"
  | "hover" | "selection"
  | "accent" | "accentInk" | "accentTint";

export const roles: readonly Role[] = [
  "chrome", "side", "content", "field", "line", "fg", "muted", "faint", "icon",
  "hover", "selection", "accent", "accentInk", "accentTint",
];

/**
 * Colour roles per theme: the single source of truth. Values start from
 * mockup A; the contrast test below is the authority, so a value may move
 * slightly to pass it. Amber (`accent`) marks attention only: dead letters
 * above zero, PROD, errors.
 */
export const themes: Record<ThemeName, Record<Role, string>> = {
  dark: {
    chrome: "#1d1f23", side: "#191b1e", content: "#141518", field: "#121316", line: "#2a2d32",
    fg: "#d9dce1", muted: "#959aa3", faint: "#727780", icon: "#9aa0a9",
    hover: "#1c1e22", selection: "#262c36",
    accent: "#f0a537", accentInk: "#1d1406", accentTint: "#372e22",
  },
  light: {
    chrome: "#eef0f3", side: "#f6f7f9", content: "#ffffff", field: "#ffffff", line: "#dde0e5",
    fg: "#1d2127", muted: "#59606b", faint: "#767d86", icon: "#59606b",
    hover: "#f3f5f8", selection: "#e2e8f1",
    accent: "#8f4f00", accentInk: "#ffffff", accentTint: "#fbf0e2",
  },
};

const surfaces: Role[] = ["chrome", "side", "content", "field", "hover", "selection"];

/** Text roles drawn on each surface, with their minimum WCAG ratio. `faint`
 *  and `icon` are never the only carrier of must-read text, so they need 3:1. */
export const contrastPairs: ReadonlyArray<{ text: Role; on: Role; min: number }> = [
  ...(["fg", "muted", "accent"] as Role[]).flatMap((text) => surfaces.map((on) => ({ text, on, min: 4.5 }))),
  ...(["faint", "icon"] as Role[]).flatMap((text) => surfaces.map((on) => ({ text, on, min: 3 }))),
  { text: "accentInk", on: "accent", min: 4.5 },
  { text: "accent", on: "accentTint", min: 4.5 },
  { text: "fg", on: "accentTint", min: 4.5 },
];
