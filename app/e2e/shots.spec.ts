import { test } from "@playwright/test";

/** Screens captured for impeccable finish reviews. Later tasks append entries. */
export const shots: { name: string; path: string }[] = [{ name: "catalogue", path: "/#catalogue" }];

for (const colorScheme of ["dark", "light"] as const) {
  for (const s of shots) {
    test(`${s.name} (${colorScheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
      await page.goto(s.path);
      await page.waitForLoadState("networkidle");
      await page.screenshot({ path: `test-results/shots/${s.name}-${colorScheme}.png`, fullPage: true });
    });
  }
}
