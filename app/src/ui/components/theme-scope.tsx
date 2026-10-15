import { useLayoutEffect, useRef, type ReactNode } from "react";
import { applyTheme, type Density, type ThemeName } from "../theme";

/** Applies a theme to one subtree (side-by-side previews). */
export function ThemeScope({ theme, density, children }: { theme: ThemeName; density: Density; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (ref.current) applyTheme(theme, density, ref.current);
  }, [theme, density]);
  return (
    <div ref={ref} className="bg-content text-fg">
      {children}
    </div>
  );
}
