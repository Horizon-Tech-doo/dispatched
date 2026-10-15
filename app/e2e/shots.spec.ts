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
      await page.keyboard.press("ArrowDown"); // keyboard focus, so the selected row carries the focus outline
      await page.keyboard.press("ArrowUp");
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
  {
    name: "entity",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      const tree = page.getByRole("tree", { name: "Namespace" });
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await tree.getByRole("treeitem", { name: "invoice-generate" }).click();
      await page.keyboard.press("Enter");
      await tree.getByRole("treeitem", { name: "shipment-events", exact: true }).click();
      await page.keyboard.press("ArrowRight");
      await tree.getByRole("treeitem", { name: "tracking-sync" }).click();
      await page.keyboard.press("Enter");
      await page.getByRole("tablist", { name: "Open entities" }).getByRole("tab", { name: /tracking-sync/ }).waitFor();
      await page.getByRole("tablist", { name: "Entity views" }).getByRole("tab", { name: /Dead-letter/ }).click();
      await page.getByRole("heading", { name: "Dead-letter isn't available yet" }).waitFor();
    },
  },
  {
    name: "entity-gone",
    path: `/?scenario=default&autoconnect=p-logistics&seed=${encodeURIComponent(
      JSON.stringify({ "shell:p-logistics": { tabs: [{ id: "deleted-queue", kind: "queue", subTab: "overview" }], activeTab: "deleted-queue" } }),
    )}`,
    ready: (p) => p.getByRole("heading", { name: "This entity wasn't found" }),
  },
  ...(["palette", "palette-all", "palette-commands", "palette-empty"] as const).map((name) => ({
    name,
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p: Page) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page: Page) => {
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.keyboard.press("ControlOrMeta+k");
      const dialog = page.getByRole("dialog", { name: "Command palette" });
      const query = { palette: "ship", "palette-all": "", "palette-commands": "", "palette-empty": "zzzz" }[name];
      if (query) await page.getByPlaceholder("Go to entity or command").fill(query);
      // Subscriptions load as the palette opens; wait for them so the shot never photographs a half-filled list.
      if (name !== "palette-empty") await dialog.getByRole("option", { name: /tracking-sync/ }).first().waitFor();
      // The commands sit below the entities: scroll the list to its end.
      if (name === "palette-commands") await dialog.getByRole("listbox").evaluate((el) => { el.scrollTop = el.scrollHeight; });
      await dialog.getByText(name === "palette-empty" ? 'No results for "zzzz"' : "Go to").first().waitFor();
    },
    holds: (p: Page) => p.getByRole("dialog", { name: "Command palette" }),
    viewportOnly: true,
  })),
  {
    name: "workspace-empty",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("heading", { name: "Open an entity" }),
  },
  {
    name: "drawer",
    path: "/?scenario=network&autoconnect=p-logistics",
    // The tree error appears after the automatic retry; the log line is checked in prepare.
    ready: (p) => p.getByRole("heading", { name: "Can't list this namespace" }),
    prepare: async (page) => {
      await page.getByRole("region", { name: "Jobs and activity" }).getByText("Listing the namespace failed").first().waitFor();
    },
  },
  {
    name: "drawer-hidden",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.keyboard.press("ControlOrMeta+j");
      await page.getByRole("button", { name: "Show activity" }).waitFor();
    },
  },
  {
    name: "drawer-jobs",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.getByRole("tab", { name: /^Jobs/ }).click();
      await page.getByText("Exports and bulk operations show their progress here").waitFor();
    },
  },
  {
    name: "profile-add",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Add connection string" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByLabel("Name", { exact: true }).fill("Orders production");
      await sheet.getByRole("button", { name: "Save" }).click(); // shows the field errors
      await sheet.getByText("Paste the namespace's connection string.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-entra",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Use a device code instead" }).click();
      await sheet.getByText("QX4T-9KLM").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-entra-signed-in",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByText("Signed in as ops@contoso.example").waitFor();
      await sheet.getByLabel("Name", { exact: true }).fill("Ops");
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-entra-discover",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByRole("listbox", { name: "Namespaces" }).waitFor();
      await sheet.getByRole("option", { name: /sb-logistics-prod/ }).waitFor();
      await sheet.getByLabel("Name", { exact: true }).fill("Logistics production");
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await sheet.getByRole("listbox", { name: "Namespaces" }).focus();
      await page.keyboard.press("ArrowDown");
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-entra-picked",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByRole("option", { name: /sb-finance-events/ }).click();
      await sheet.getByRole("group", { name: "Selected namespace" }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-entra-mfa",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByRole("listbox", { name: "Namespaces" }).waitFor();
      await sheet.getByRole("combobox", { name: "Tenant" }).click();
      await page.getByRole("option", { name: "Northwind Traders" }).click();
      await sheet.getByRole("button", { name: "Sign in to Northwind Traders" }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-edit",
    path: "/?scenario=default",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Logistics", exact: true }).click();
      const sheet = page.getByRole("dialog", { name: "Edit Logistics", exact: true });
      await sheet.getByRole("button", { name: "Test connection" }).click();
      await sheet.getByText(/listed 4 queue/).waitFor();
      await sheet.getByRole("button", { name: "Delete…" }).click();
      await sheet.getByRole("button", { name: "Delete Logistics" }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Edit Logistics", exact: true }),
    viewportOnly: true,
  },
  {
    name: "profile-edit-changed",
    path: "/?scenario=default",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Logistics", exact: true }).click();
      const sheet = page.getByRole("dialog", { name: "Edit Logistics", exact: true });
      await sheet.getByLabel("Proxy").fill("http://proxy.corp:8080");
      await sheet.getByText("Save to test these changes.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Edit Logistics", exact: true }),
    viewportOnly: true,
  },
  {
    name: "profile-edit-entra",
    path: "/?scenario=default",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Logistics dev", exact: true }).click();
      const sheet = page.getByRole("dialog", { name: "Edit Logistics dev", exact: true });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByRole("button", { name: "Use another account" }).click();
      await sheet.getByText(/This connection uses a different account/).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Edit Logistics dev", exact: true }),
    viewportOnly: true,
  },
  {
    name: "profiles",
    path: "/?scenario=default",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Logistics", exact: true }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Connections" }),
    viewportOnly: true,
  },
  {
    name: "settings",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.keyboard.press("ControlOrMeta+,");
      await page.getByRole("dialog", { name: "Settings" }).getByText("Row height in the tree and grids.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Settings" }),
    viewportOnly: true,
  },
  {
    name: "settings-comfortable",
    path: `/?scenario=default&autoconnect=p-logistics&seed=${encodeURIComponent(JSON.stringify({ ui: { theme: "system", density: "comfortable", autoRefresh: 30 } }))}`,
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.keyboard.press("ControlOrMeta+,");
      await page.getByRole("dialog", { name: "Settings" }).getByText("Row height in the tree and grids.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Settings" }),
    viewportOnly: true,
  },
  ...(["update", "update-downloading"] as const).map((name) => ({
    name,
    path: `/?scenario=default&autoconnect=p-logistics&update=${name === "update" ? "1" : "downloading"}`,
    ready: (p: Page) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page: Page) => {
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("button", { name: "Update 0.1.0 available" }).click(); // appears 3 s after start
      const sheet = page.getByRole("dialog", { name: "Updates" });
      if (name === "update-downloading") {
        await sheet.getByRole("button", { name: "Install and restart" }).click();
        await sheet.getByRole("progressbar").and(page.locator("[aria-valuenow=\"60\"]")).waitFor();
      } else {
        await sheet.getByText("Faster tree loading.").waitFor();
      }
    },
    holds: (p: Page) => p.getByRole("dialog", { name: "Updates" }),
    viewportOnly: true,
  })),
  {
    name: "update-notice",
    path: "/?scenario=default&autoconnect=p-logistics&update=1",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Update 0.1.0 available" }).waitFor(); // appears 3 s after start
    },
  },
  {
    name: "update-check-error",
    path: "/?scenario=default&autoconnect=p-logistics&update=check-error",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.keyboard.press("ControlOrMeta+,");
      await page.getByRole("dialog", { name: "Settings" }).getByRole("button", { name: "Check for updates" }).click();
      await page.getByRole("dialog", { name: "Updates" }).getByText("Could not reach the update server.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Updates" }),
    viewportOnly: true,
  },
  {
    name: "update-install-error",
    path: "/?scenario=default&autoconnect=p-logistics&update=install-error",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Update 0.1.0 available" }).click(); // appears 3 s after start
      const sheet = page.getByRole("dialog", { name: "Updates" });
      await sheet.getByRole("button", { name: "Install and restart" }).click();
      await sheet.getByText("The download was interrupted.").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Updates" }),
    viewportOnly: true,
  },
  {
    name: "update-uptodate",
    path: "/?scenario=default&autoconnect=p-logistics",
    ready: (p) => p.getByRole("tree", { name: "Namespace" }).getByRole("treeitem", { name: "invoice-generate" }),
    prepare: async (page) => {
      await page.keyboard.press("ControlOrMeta+,");
      await page.getByRole("dialog", { name: "Settings" }).getByRole("button", { name: "Check for updates" }).click();
      await page.getByRole("dialog", { name: "Updates" }).getByText(/^Dispatched 0\.0\.1-mock\. Checked/).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Updates" }),
    viewportOnly: true,
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
