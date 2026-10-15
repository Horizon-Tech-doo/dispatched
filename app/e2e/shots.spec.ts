import { test } from "@playwright/test";

/** Screens captured for impeccable finish reviews. Later tasks append entries.
 *  `scroll: true` marks a screen whose content scrolls inside an `[data-shot-scroller]` element
 *  (the page itself never scrolls), so the capture expands it to its full height. */
export const shots: { name: string; path: string; scroll?: boolean }[] = [
  { name: "catalogue", path: "/#catalogue", scroll: true },
];

for (const colorScheme of ["dark", "light"] as const) {
  for (const s of shots) {
    test(`${s.name} (${colorScheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
      await page.goto(s.path);
      await page.waitForLoadState("networkidle");
      if (s.scroll) {
        // body is overflow:hidden with a fixed-height root; let the page grow to the scroller's content.
        await page.addStyleTag({
          content:
            "html, body, #root, [data-shot-scroller] { height: auto !important; overflow: visible !important; } html { background: var(--c-content) !important; }",
        });
      }
      await page.screenshot({ path: `test-results/shots/${s.name}-${colorScheme}.png`, fullPage: true });
    });
  }
}
