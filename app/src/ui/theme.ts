import { useEffect, useState } from "react";
import { roles, themes, type Role, type ThemeName } from "./tokens";

export type { ThemeName } from "./tokens";
export type ThemePref = "system" | ThemeName;
export type Density = "compact" | "comfortable";

export const rowHeight: Record<Density, number> = { compact: 22, comfortable: 26 };

const cssVar = (role: Role) => `--c-${role.replace(/[A-Z]/g, (c) => `-${c.toLowerCase()}`)}`;

export function resolveTheme(pref: ThemePref, systemDark: boolean): ThemeName {
  return pref === "system" ? (systemDark ? "dark" : "light") : pref;
}

export function applyTheme(name: ThemeName, density: Density, root: HTMLElement = document.documentElement): void {
  for (const role of roles) root.style.setProperty(cssVar(role), themes[name][role]);
  root.style.setProperty("--row", `${rowHeight[density]}px`);
  root.style.colorScheme = name;
  root.dataset.theme = name;
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
