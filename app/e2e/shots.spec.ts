import { test, type Page } from "@playwright/test";

/** Screens captured for impeccable finish reviews. Later tasks append entries.
 *  `scroll: true` marks a screen whose content scrolls inside an `[data-shot-scroller]` element
 *  (the page itself never scrolls), so the capture expands it to its full height.
 *  `prepare` runs after load and before the screenshot (open a sheet, a menu...). */
export const shots: { name: string; path: string; scroll?: boolean; prepare?: (page: Page) => Promise<void> }[] = [
  { name: "catalogue", path: "/#catalogue", scroll: true },
  {
    name: "catalogue-tree",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Tree" }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("tree", { name: "Explorer" }).first().focus();
    },
  },
  {
    name: "catalogue-grid",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Grid" }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("grid", { name: "Dead-lettered messages" }).first().focus();
    },
  },
  {
    name: "catalogue-overlays",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Overlays" }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.getByRole("button", { name: "Open sheet" }).first().click();
      await page.getByRole("dialog", { name: "Add connection" }).waitFor();
    },
  },
];

for (const colorScheme of ["dark", "light"] as const) {
  for (const s of shots) {
    test(`${s.name} (${colorScheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
      await page.goto(s.path);
      await page.waitForLoadState("networkidle");
      await s.prepare?.(page);
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
