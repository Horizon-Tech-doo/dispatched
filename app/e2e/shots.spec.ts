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

/** A connected logistics workbench with one entity tab open on `subTab` (drawer closed, so the view has the room). */
const seedTab = (id: string, kind: "queue" | "subscription", subTab: string, scenario = "default", extra: Record<string, unknown> = {}) =>
  `/?scenario=${scenario}&autoconnect=p-logistics&seed=${encodeURIComponent(
    JSON.stringify({
      ...extra,
      "shell:p-logistics": {
        tabs: [{ id, kind, subTab }], activeTab: id, expanded: ["group:queues", "group:topics"], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
      },
    }),
  )}`;
const peekStatus = (p: Page) => p.getByRole("status", { name: "Peek status" });

type Shot = {
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
};

const TRACKING = "shipment-events/subscriptions/tracking-sync";
const DETAIL_VIEW = () => seedTab(TRACKING, "subscription", "dead-letter");
/** Select the grid's `row`-th data row (0-based) and open the detail pane with Enter, then wait for its body. */
const openRow = async (page: Page, row: number, body: string | RegExp = "Body") => {
  await peekStatus(page).getByText(/100 messages/).waitFor();
  await page.locator("[data-row-id]").nth(row).click();
  await page.getByRole("grid").press("Enter");
  await page.getByRole("region", { name: body }).first().waitFor();
};

/** Read from the end, select the row `up` rows above the newest and open the pane on it. */
const openFromEnd = async (page: Page, up: number) => {
  await peekStatus(page).getByText(/100 messages/).waitFor();
  await page.getByRole("radio", { name: "End" }).click();
  await peekStatus(page).getByText("Last 100 messages").waitFor();
  await page.getByRole("grid").focus();
  await page.keyboard.press("End");
  for (let i = 0; i < up; i++) await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Enter");
};

/** The message detail pane. The mock's bodies by position: row 0 is binary (not UTF-8), 1 JSON, 2 XML, 3 text. */
function detailShots(): Shot[] {
  const pane = (p: Page) => p.getByRole("region", { name: /^Message \d+$/ });
  return [
    { name: "message-detail-json", path: DETAIL_VIEW(), prepare: (p) => openRow(p, 1) },
    { name: "message-detail-xml", path: DETAIL_VIEW(), prepare: (p) => openRow(p, 2) },
    { name: "message-detail-hex", path: DETAIL_VIEW(), prepare: (p) => openRow(p, 0, "Hex") },
    {
      name: "message-detail-properties",
      path: DETAIL_VIEW(),
      prepare: async (p) => {
        await openRow(p, 1);
        await pane(p).getByRole("tab", { name: "Properties" }).click();
        await pane(p).getByRole("table", { name: "Custom properties" }).waitFor();
      },
    },
    {
      // The newest dead letters of tracking-sync: 48213 is a 5 MiB binary cut to a few thousand rows, 48212 a 6 MiB log cut the
      // same way, 48211 has no body.
      name: "message-detail-truncated",
      path: DETAIL_VIEW(),
      prepare: async (p) => {
        await openFromEnd(p, 0);
        await p.getByText(/^Showing the first .* of 5\.0 MiB/).waitFor();
      },
    },
    {
      name: "message-detail-text-truncated",
      path: DETAIL_VIEW(),
      prepare: async (p) => {
        await openFromEnd(p, 1);
        await p.getByText(/^Showing the first .* of 6\.0 MiB/).waitFor();
      },
    },
    {
      name: "message-detail-empty",
      path: DETAIL_VIEW(),
      prepare: async (p) => {
        await openFromEnd(p, 2);
        await p.getByText("Empty body (0 bytes)").waitFor();
      },
    },
    {
      name: "message-detail-none",
      path: DETAIL_VIEW(),
      prepare: async (p) => {
        await peekStatus(p).getByText(/100 messages/).waitFor();
        // The pane is open (Enter or a double click did it) but nothing is selected: put the view in that state.
        await p.evaluate(() => {
          const m = (window as unknown as { __dispatchedMock: { setViewState(k: string, d: unknown, p: unknown): void; viewKey(p: string, t: string, v: string): string } }).__dispatchedMock;
          const defaults = { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: false, sorting: null, detailOpen: false };
          m.setViewState(m.viewKey("p-logistics", "shipment-events/subscriptions/tracking-sync", "dead-letter"), defaults, { detailOpen: true });
        });
        await p.getByRole("heading", { name: "No message selected" }).waitFor();
      },
    },
    {
      // An active view: the Body preview column is on, and the pane sits beside it.
      name: "message-detail-active",
      path: seedTab("customs-docs", "queue", "messages"),
      prepare: (p) => openRow(p, 1),
    },
    {
      name: "message-detail-gone",
      path: seedTab(TRACKING, "subscription", "dead-letter", "message-gone"),
      prepare: async (p) => {
        await openRow(p, 1, /^Message \d+$/);
        await p.getByRole("heading", { name: "This message is no longer in the dead-letter queue" }).waitFor();
      },
    },
  ];
}

/** The export sheet and the Jobs tab. */
function exportShots(): Shot[] {
  const INVOICES = (scenario = "default") => seedTab("invoice-generate", "queue", "dead-letter", scenario);
  const sheet = (p: Page) => p.getByRole("dialog", { name: "Export messages" });
  const openExport = async (page: Page, item: string | RegExp) => {
    await page.getByRole("button", { name: "Export", exact: true }).click();
    await page.getByRole("menuitem", { name: item }).click();
    await sheet(page).waitFor();
  };
  const jobs = (p: Page) => p.getByRole("region", { name: "Jobs and activity" });
  /** Export the whole view and wait for the job's row. */
  const exportAll = async (page: Page) => {
    await openExport(page, "Export all…");
    await sheet(page).getByRole("button", { name: /^Export 37 messages…/ }).click();
    await jobs(page).getByRole("listitem").waitFor();
  };
  /** Put the view in a state no control reaches yet (a filter set before the sheet opens). */
  const setFilter = (page: Page, filter: Record<string, string>) =>
    page.evaluate((f) => {
      const m = (window as unknown as { __dispatchedMock: { setViewState(k: string, d: unknown, p: unknown): void; viewKey(p: string, t: string, v: string): string } }).__dispatchedMock;
      const defaults = { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: false, sorting: null, detailOpen: false };
      m.setViewState(m.viewKey("p-logistics", "invoice-generate", "dead-letter"), defaults, { filter: f });
    }, filter);
  const selectRows = async (page: Page, n: number) => {
    await page.getByRole("grid").focus();
    await page.keyboard.press("Home");
    for (let i = 1; i < n; i++) await page.keyboard.press("Shift+ArrowDown");
  };
  return [
    {
      // Three rows selected, so both scopes are offered.
      name: "export-sheet",
      path: INVOICES(),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await selectRows(page, 3);
        await openExport(page, /^Export 3 selected/);
      },
      holds: sheet,
      viewportOnly: true,
    },
    {
      // A file chosen, then the request refused (a filter that does not compile): the file is kept and named.
      name: "export-sheet-file-chosen",
      path: INVOICES(),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await setFilter(page, { body_regex: "(" });
        await openExport(page, "Export all…");
        await sheet(page).getByRole("button", { name: "Export messages…" }).click();
        await sheet(page).getByRole("alert").waitFor();
        await sheet(page).getByRole("button", { name: "Choose another file" }).waitFor();
      },
      holds: sheet,
      viewportOnly: true,
    },
    {
      name: "jobs-running",
      path: INVOICES("export-slow"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await exportAll(page);
        // Let a few messages through so the bar and the count show progress.
        await jobs(page).getByText(/^([3-9]|\d\d) of 37$/).waitFor();
      },
    },
    {
      name: "jobs-done",
      path: INVOICES(),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await exportAll(page);
        await jobs(page).getByText("37 messages written", { exact: true }).waitFor();
        await jobs(page).getByRole("button", { name: /^Show in/ }).waitFor();
      },
    },
    {
      // Four selected, two of them gone by the time the export reads them again.
      name: "jobs-missing",
      path: INVOICES("export-missing"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await selectRows(page, 4);
        await openExport(page, /^Export 4 selected/);
        await sheet(page).getByRole("button", { name: "Export 4 messages…" }).click();
        await jobs(page).getByText("2 selected messages were no longer there").waitFor();
      },
    },
    {
      // A filter, so no total: no bar fill, what was written and how many were examined.
      name: "jobs-unknown-total",
      path: INVOICES("export-slow"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await setFilter(page, { body_contains: "SHP" });
        await openExport(page, "Export all…");
        await sheet(page).getByRole("button", { name: "Export messages…" }).click();
        await jobs(page).getByText(/ written · .* scanned$/).waitFor();
      },
    },
    {
      name: "jobs-cancelled",
      path: INVOICES("export-slow"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await exportAll(page);
        await jobs(page).getByText(/^([3-9]|\d\d) of 37$/).waitFor();
        await jobs(page).getByRole("button", { name: /^Cancel Export/ }).click();
        await jobs(page).getByText(/^Cancelled after \d+ of 37\. No file was written\.$/).waitFor();
      },
    },
    {
      name: "jobs-failed",
      path: INVOICES("export-fails"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await exportAll(page);
        await jobs(page).getByText("Export failed").waitFor();
        await jobs(page).getByRole("button", { name: "Try again" }).waitFor();
      },
    },
    {
      // The drawer at a narrow window: the row stacks (title, bar, numbers and action).
      name: "jobs-narrow",
      path: INVOICES("export-slow"),
      ready: (p) => peekStatus(p).getByText(/37 messages/),
      prepare: async (page) => {
        await page.setViewportSize({ width: 640, height: 800 });
        await exportAll(page);
        await jobs(page).getByText(/^([3-9]|\d\d) of 37$/).waitFor();
      },
    },
  ];
}

export const shots: Shot[] = [
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
      // The light column's grid, so the focus ring shows in light here and in dark in catalogue-grid-sorted.
      await page.getByRole("grid", { name: "Dead-lettered messages", exact: true }).last().focus();
    },
  },
  {
    name: "catalogue-grid-sorted",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Grid" }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      // The catalogue renders a dark and a light copy side by side: sort both.
      for (const grid of await page.getByRole("grid", { name: "Dead-lettered messages", exact: true }).all()) {
        const seq = grid.getByRole("button", { name: "Seq" });
        // By keyboard, so the pointer never hovers a header and :focus-visible still matches at the end.
        await seq.focus();
        await page.keyboard.press("Enter");
        await page.keyboard.press("Enter");
      }
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("grid", { name: "Dead-lettered messages", exact: true }).first().focus();
    },
  },
  {
    name: "catalogue-viewer",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Viewer", exact: true }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("region", { name: "JSON body" }).first().focus();
    },
  },
  {
    name: "catalogue-viewer-states",
    path: "/#catalogue",
    prepare: async (page) => {
      await page.getByRole("heading", { name: "Viewer states", exact: true }).first().evaluate((el) => el.scrollIntoView({ block: "start" }));
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.getByRole("region", { name: "Long line" }).first().focus();
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
  {
    name: "connect-tenant-sign-in",
    path: "/?scenario=tenant-sign-in&autoconnect=p-northwind",
    ready: (p) => p.getByRole("button", { name: "Sign in to Northwind Traders" }),
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
      await page.getByRole("status", { name: "Peek status" }).getByText(/100 messages/).waitFor();
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
    name: "sign-in-device-code",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Use a device code instead" }).click();
      await sheet.getByRole("button", { name: "Open sign-in page" }).waitFor();
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
    name: "profile-entra-picked-guest",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Sign in with Entra ID" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByRole("button", { name: "Sign in with Entra ID" }).click();
      await sheet.getByRole("listbox", { name: "Namespaces" }).waitFor();
      await sheet.getByRole("combobox", { name: "Tenant" }).click();
      await page.getByRole("option", { name: "Northwind Traders" }).click();
      await sheet.getByRole("button", { name: "Sign in to Northwind Traders" }).click();
      await sheet.getByText("Listed with your own sign-in to this tenant, as ops@contoso.example.").waitFor();
      await sheet.getByRole("option", { name: /nw-orders/ }).click();
      await sheet.getByRole("group", { name: "Selected namespace" }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-test-draft",
    path: "/?scenario=empty",
    ready: (p) => p.getByRole("heading", { name: "Add a connection" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Add connection string" }).click();
      const sheet = page.getByRole("dialog", { name: "Add connection" });
      await sheet.getByLabel("Name", { exact: true }).fill("Orders production");
      await sheet.getByLabel("Connection string").fill("Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0");
      await sheet.getByRole("button", { name: "Test connection" }).click();
      await sheet.getByRole("status").getByText(/^Tested /).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Add connection" }),
    viewportOnly: true,
  },
  {
    name: "profile-test-failed",
    // One check failed (Management), so the other was skipped.
    path: "/?scenario=forbidden",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Logistics", exact: true }).click();
      const sheet = page.getByRole("dialog", { name: "Edit Logistics", exact: true });
      await sheet.getByRole("button", { name: "Test connection" }).click();
      await sheet.getByText("Messaging not tested").waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Edit Logistics", exact: true }),
    viewportOnly: true,
  },
  {
    name: "profile-edit-tenant",
    path: "/?scenario=tenant-sign-in",
    ready: (p) => p.getByRole("list", { name: "Saved connections" }),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Connection: Choose a connection" }).click();
      await page.getByRole("menuitem", { name: "Manage connections…" }).click();
      await page.getByRole("dialog", { name: "Connections" }).getByRole("button", { name: "Edit Northwind orders", exact: true }).click();
      const sheet = page.getByRole("dialog", { name: "Edit Northwind orders", exact: true });
      await sheet.getByRole("button", { name: "Sign in to Northwind Traders" }).waitFor();
    },
    holds: (p) => p.getByRole("dialog", { name: "Edit Northwind orders", exact: true }),
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
      await sheet.getByRole("button", { name: "Test connection" }).click();
      await sheet.getByRole("status").getByText(/^Tested /).waitFor();
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
    name: "settings-accounts",
    // Two kept sign-ins: the current account's (used by two connections, one of them connected) and another
    // account's whose token is gone.
    path: "/?scenario=tenant-sign-in&autoconnect=p-logistics-dev",
    ready: (p) => p.getByRole("button", { name: /^Connection: Logistics dev/ }),
    prepare: async (page) => {
      await page.keyboard.press("Shift"); // keyboard modality so :focus-visible matches
      await page.keyboard.press("ControlOrMeta+,");
      const sheet = page.getByRole("dialog", { name: "Settings" });
      const row = sheet.getByRole("listitem", { name: "ops@contoso.example" });
      await row.getByText(/Used by/).waitFor();
      await row.getByRole("button", { name: "Sign out" }).click();
      await row.getByText(/is connected and will disconnect/).waitFor();
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
  {
    name: "messages-dlq",
    path: seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/100 messages/),
  },
  {
    name: "messages-active-sessions",
    path: seedTab("returns-intake", "queue", "messages"),
    ready: (p) => peekStatus(p).getByText(/48 messages/),
  },
  {
    name: "messages-empty",
    path: seedTab("label-print", "queue", "dead-letter"),
    ready: (p) => p.getByRole("heading", { name: "No dead letters" }),
  },
  {
    name: "messages-error",
    path: seedTab("invoice-generate", "queue", "dead-letter", "peek-error"),
    ready: (p) => p.getByRole("heading", { name: "Can't read the dead-letter queue" }),
  },
  {
    name: "messages-from-end",
    path: seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/100 messages/),
    prepare: async (page) => {
      await page.getByRole("radio", { name: "End" }).click();
      await peekStatus(page).getByText("Last 100 messages").waitFor();
    },
  },
  {
    name: "messages-sorted",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      // By keyboard, so the pointer never hovers a header.
      await page.getByRole("grid").getByRole("button", { name: "Seq" }).focus();
      await page.keyboard.press("Enter");
      await page.keyboard.press("Enter");
      await page.getByRole("columnheader", { name: /Seq/ }).and(page.locator('[aria-sort="descending"]')).waitFor();
      await page.getByRole("grid").focus();
    },
  },
  {
    name: "messages-streaming",
    path: seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter", "peek-slow", { ui: { peekSize: 1000 } }),
    ready: (p) => peekStatus(p).getByText("500 of 1,000 · reading…"),
  },
  {
    name: "messages-columns-menu",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Columns" }).click();
      await page.getByRole("menuitemcheckbox", { name: "Description" }).waitFor();
    },
    holds: (p) => p.getByRole("menu"),
    viewportOnly: true,
  },
  {
    name: "messages-filter",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Filter" }).click();
      const bar = page.getByRole("search", { name: "Message filter" });
      await bar.getByLabel("Properties").fill("attempt >= 2");
      await bar.getByLabel("Body contains").fill("held");
      await bar.getByLabel("Body contains").press("Enter");
      await peekStatus(page).getByText(/matching messages? · scanned 37/).waitFor();
      // The pointer rests on nothing, and focus is in the grid, not on a field.
      await page.getByRole("grid").focus();
    },
  },
  {
    name: "messages-filter-collapsed",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Filter" }).click();
      const bar = page.getByRole("search", { name: "Message filter" });
      await bar.getByLabel("Properties").fill("attempt >= 2");
      await bar.getByLabel("Body contains").fill("held");
      await bar.getByLabel("Body contains").press("Enter");
      await peekStatus(page).getByText(/matching messages? · scanned 37/).waitFor();
      await bar.getByLabel("Body contains").press("Escape");
      await page.getByRole("search", { name: "Message filter" }).waitFor({ state: "detached" });
      await page.getByRole("grid").focus();
    },
  },
  {
    name: "messages-filter-json-session",
    path: seedTab("returns-intake", "queue", "messages"),
    ready: (p) => peekStatus(p).getByText(/48 messages/),
    prepare: async (page) => {
      // A session condition has no control to set it yet (Sessions comes later): put it in the view's state.
      await page.evaluate(() => {
        const m = (window as unknown as { __dispatchedMock: { setViewState(k: string, d: unknown, p: unknown): void; viewKey(p: string, t: string, v: string): string } }).__dispatchedMock;
        const defaults = { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: true, sorting: null, detailOpen: false };
        m.setViewState(m.viewKey("p-logistics", "returns-intake", "active"), defaults, { filterOpen: true, filter: { session_id: "RMA-1010" } });
      });
      const bar = page.getByRole("search", { name: "Message filter" });
      await bar.getByRole("radio", { name: "JSONPath" }).click();
      await bar.getByLabel("JSONPath").fill("$.status");
      await bar.getByLabel("Equals").fill("in_transit");
      await bar.getByLabel("Equals").press("Enter");
      await page.getByRole("button", { name: "Filter 2", exact: true }).waitFor();
      await peekStatus(page).getByText(/matching messages? · scanned/).waitFor();
      await page.getByRole("grid").focus();
    },
  },
  {
    name: "messages-filter-error",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Filter" }).click();
      const bar = page.getByRole("search", { name: "Message filter" });
      await bar.getByLabel("Properties").fill("sys.Subject == 'x'");
      await bar.getByText("Use = to compare.", { exact: true }).first().waitFor();
    },
  },
  {
    name: "messages-filter-empty",
    path: seedTab("invoice-generate", "queue", "dead-letter"),
    ready: (p) => peekStatus(p).getByText(/37 messages/),
    prepare: async (page) => {
      await page.getByRole("button", { name: "Filter" }).click();
      const bar = page.getByRole("search", { name: "Message filter" });
      await bar.getByLabel("Body contains").fill("no-such-text");
      await bar.getByLabel("Body contains").press("Enter");
      await page.getByRole("heading", { name: "No messages match this filter" }).waitFor();
      await page.getByRole("grid").focus();
    },
  },
  ...detailShots(),
  ...exportShots(),
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
