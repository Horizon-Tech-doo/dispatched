import { expect, test, type Locator, type Page } from "@playwright/test";

/** Screens captured for impeccable finish reviews. Later tasks append entries.
 *  `scroll: true` marks a screen whose content scrolls inside an `[data-shot-scroller]` element
 *  (the page itself never scrolls), so the capture expands it to its full height.
 *  `prepare` runs after load and before the screenshot (open a sheet, a menu...). */
const onScreen = async (page: Page, el: Locator) => {
  const box = await el.boundingBox().catch(() => null);
  const vp = page.viewportSize();
  return box != null && vp != null && box.y >= 0 && box.x >= 0 && box.y + box.height <= vp.height && box.x + box.width <= vp.width;
};

export const shots: {
  name: string;
  path: string;
  scroll?: boolean;
  /** Content that must be on screen before the capture (so mock-data loading is never photographed). */
  ready?: (page: Page) => Locator;
  prepare?: (page: Page) => Promise<void>;
  /** Transient UI (an open menu) that must be on screen at the moment of capture, and is re-opened if it closed. */
  holds?: (page: Page) => Locator;
  /** Capture the viewport only; for entries whose transient UI a full-page resize could dismiss. */
  viewportOnly?: boolean;
}[] = [
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
  { name: "welcome", path: "/?scenario=default", ready: (p) => p.getByRole("list", { name: "Saved connections" }) },
  { name: "welcome-empty", path: "/?scenario=empty", ready: (p) => p.getByRole("heading", { name: "Add a connection" }) },
  {
    name: "connect-failed",
    path: "/?scenario=missing-secret&autoconnect=p-logistics",
    ready: (p) => p.getByRole("heading", { name: "Couldn't connect to Logistics" }),
  },
  { name: "shell-prod", path: "/?scenario=default&autoconnect=p-logistics", ready: (p) => p.getByRole("banner").getByText("PROD", { exact: true }) },
  {
    name: "shell-selector",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("banner").getByText("PROD", { exact: true }),
    prepare: async (page) => {
      await page.getByRole("button", { name: /^Connection:/ }).click();
    },
    holds: (p) => p.getByRole("menu"),
    viewportOnly: true,
  },
  {
    name: "tree",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      const tree = page.getByRole("tree", { name: "Namespace" });
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      for (const topic of ["archive-events", "carrier-updates", "stock-levels", "shipment-events"]) {
        await tree.getByRole("treeitem", { name: topic, exact: true }).click();
        await page.keyboard.press("ArrowRight");
      }
      await tree.getByRole("treeitem", { name: "replenish" }).waitFor(); // every subscription list has loaded
      await tree.getByRole("treeitem", { name: "No subscriptions" }).waitFor();
      await tree.getByRole("treeitem", { name: "tracking-sync" }).click();
      await expect(tree.getByRole("treeitem", { name: "tracking-sync" })).toHaveAttribute("aria-selected", "true");
      // WebKit can photograph the row highlight one frame late; let it paint.
      await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    },
  },
  {
    name: "tree-menu",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }).click({ button: "right" });
    },
    holds: (p) => p.getByRole("menu"),
    viewportOnly: true,
  },
  {
    name: "tree-forbidden",
    path: "/?scenario=forbidden&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "orders" }),
  },
  {
    name: "tree-error",
    path: "/?scenario=network&autoconnect=p-logistics",
    ready: (p) => p.getByRole("heading", { name: "Can't list this namespace" }),
  },
  {
    name: "tree-many",
    path: "/?scenario=many&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "queue-0000" }),
  },
];

for (const colorScheme of ["dark", "light"] as const) {
  for (const s of shots) {
    test(`${s.name} (${colorScheme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
      await page.goto(s.path);
      await page.waitForLoadState("networkidle");
      await s.ready?.(page).waitFor();
      const capture = async () => {
        await s.prepare?.(page);
        if (s.scroll) {
          // body is overflow:hidden with a fixed-height root; let the page grow to the scroller's content.
          await page.addStyleTag({
            content:
              "html, body, #root, [data-shot-scroller] { height: auto !important; overflow: visible !important; } html { background: var(--c-content) !important; }",
          });
        }
        if (s.holds) {
          await expect(s.holds(page)).toBeVisible();
          // Radix mounts a popper off-screen until it has measured it; "visible" alone can still photograph nothing.
          await expect.poll(() => onScreen(page, s.holds!(page))).toBe(true);
        }
        await page.screenshot({ path: `test-results/shots/${s.name}-${colorScheme}.png`, fullPage: !s.viewportOnly });
      };
      await capture();
      // A transient surface that vanished during the capture means the shot is wrong: retry, then fail loudly.
      for (let attempt = 0; s.holds && !(await onScreen(page, s.holds(page))); attempt++) {
        expect(attempt, `${s.name}: the held UI closed before the screenshot`).toBeLessThan(3);
        await page.keyboard.press("Escape");
        await capture();
      }
    });
  }
}
