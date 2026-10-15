import { useEffect, useState } from "react";
import { labelColors, roles, themes, type Role, type ThemeName } from "./tokens";

export type { ThemeName } from "./tokens";
export type ThemePref = "system" | ThemeName;
export type Density = "compact" | "comfortable";

export const rowHeight: Record<Density, number> = { compact: 22, comfortable: 26 };

const cssVar = (role: Role) => `--c-${role.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;

export function resolveTheme(pref: ThemePref, systemDark: boolean): ThemeName {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

export function applyTheme(themeName: ThemeName, density: Density, root: HTMLElement = document.documentElement): void {
  for (const role of roles) root.style.setProperty(cssVar(role), themes[themeName][role]);
  for (const [name, c] of Object.entries(labelColors)) root.style.setProperty(`--label-${name}`, c[themeName]);
  root.style.setProperty("--row", `${rowHeight[density]}px`);
  root.style.colorScheme = themeName;
  root.dataset.theme = themeName;
  root.dataset.density = density;
}

export function useSystemDark(): boolean {
  const query = "(prefers-color-scheme: dark)";
  const [dark, setDark] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mql = window.matchMedia(query);
    const onChange = (e: MediaQueryListEvent) => setDark(e.matches);
    mql.addEventListener("change", onChange);
    return () => mql.removeEventListener("change", onChange);
  }, []);
  return dark;
}

/** Applies the theme to <html> and returns the resolved name. */
export function useTheme(pref: ThemePref, density: Density): ThemeName {
  const name = resolveTheme(pref, useSystemDark());
  useEffect(() => applyTheme(name, density), [name, density]);
  return name;
}
