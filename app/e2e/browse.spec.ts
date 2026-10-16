import { expect, test, type Page } from "@playwright/test";

/** A connected Logistics workbench with one entity tab open on `subTab` and the drawer open on Activity. */
const seed = (id: string, kind: "queue" | "topic" | "subscription" | "dashboard", subTab: string, extra: Record<string, unknown> = {}) =>
  encodeURIComponent(
    JSON.stringify({
      "shell:p-logistics": {
        tabs: [{ id, kind, subTab }],
        activeTab: id,
        expanded: ["group:queues", "group:topics"],
        selected: null,
        layout: { treeSize: 18, drawerOpen: true, drawerSize: 22, drawerTab: "activity" },
        ...extra,
      },
    }),
  );
const open = (page: Page, scenario: string, id?: string, kind?: "queue" | "topic" | "subscription", subTab = "overview", extra?: Record<string, unknown>) =>
  page.goto(`/?scenario=${scenario}&autoconnect=p-logistics${id ? `&seed=${seed(id, kind!, subTab, extra)}` : ""}`);

const INVOICES = "invoice-generate";
const TRACKING = "shipment-events/subscriptions/tracking-sync";

const status = (page: Page) => page.getByRole("status", { name: "Peek status" });
const drawer = (page: Page) => page.getByRole("region", { name: "Jobs and activity" });
const appStatus = (page: Page) => page.getByRole("contentinfo");
const jobUpdates = (page: Page) => page.getByRole("status", { name: "Job updates" });

/** Export the whole view: the menu, the sheet's primary (which opens the save dialog), and the job's row. */
async function exportAll(page: Page, label: RegExp | string) {
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Export all…" }).click();
  await page.getByRole("dialog", { name: "Export messages" }).getByRole("button", { name: label }).click();
  await expect(drawer(page).getByRole("listitem")).toHaveCount(1);
}

test("dead letters: rows, detail, filter, and what the status bar promises", async ({ page }) => {
  await open(page, "default", INVOICES, "queue", "dead-letter");
  const grid = page.getByRole("grid", { name: "Dead-letter queue" });
  await expect(grid.getByText("MaxDeliveryCountExceeded").first()).toBeVisible();
  await expect(status(page)).toContainText("37 messages");
  // The promise lives in the window's status bar, not in the view's own status line.
  await expect(appStatus(page).getByText("Peek only, nothing locked")).toBeVisible();
  await expect(status(page)).not.toContainText("Peek only");

  // Enter on a selected row opens the pane. Escape with a control in the pane focused closes it and hands the keyboard
  // back to the grid; so does the Close message button.
  await grid.getByRole("row").nth(1).click();
  await expect(grid.getByRole("row").nth(1)).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Enter");
  const detail = page.getByRole("region", { name: /^Message \d+$/ });
  await expect(detail).toBeVisible();
  const copyJson = detail.getByRole("button", { name: "Copy JSON" });
  await expect(copyJson).toBeEnabled();
  // A tab, not Copy JSON: a focused button shows its tooltip, and Escape dismisses that first.
  await detail.getByRole("tab", { name: "Body" }).focus();
  await page.keyboard.press("Escape");
  await expect(detail).toHaveCount(0);
  await expect(grid).toBeFocused();
  await page.keyboard.press("Enter");
  await detail.getByRole("button", { name: "Close message" }).click();
  await expect(detail).toHaveCount(0);
  await expect(grid).toBeFocused();

  // The filter bar: a body condition, applied with Enter. The view says what matched and how much it examined.
  await page.getByRole("button", { name: "Filter", exact: true }).click();
  const bar = page.getByRole("search", { name: "Message filter" });
  await bar.getByLabel("Body contains").fill("SHP-1000");
  await bar.getByLabel("Body contains").press("Enter");
  // The mock's dead-letter queue has 9 messages with SHP-1000 in the body, out of the 37 it examines.
  await expect(status(page)).toHaveText("9 matching messages · scanned 37 · end of the dead-letter queue");
  await expect(page.locator("[data-row-id]")).toHaveCount(9);

  // Closed, the filter stays visible as a summary, and one click clears it.
  await bar.getByLabel("Body contains").press("Escape");
  await expect(bar).toHaveCount(0);
  await page.getByRole("button", { name: /^Edit filter: / }).waitFor();
  await page.getByRole("button", { name: "Clear filter" }).click();
  await expect(status(page)).toContainText("37 messages");
});

test("12,000 messages stay navigable from the keyboard", async ({ page }) => {
  test.slow();
  await open(page, "many-messages", "customs-docs", "queue", "messages");
  await page.getByRole("combobox", { name: "Messages to read" }).click();
  await page.getByRole("option", { name: "50,000" }).click();
  await expect(status(page)).toContainText("12,000 messages", { timeout: 30_000 });
  await expect(status(page)).toContainText("end of the queue");
  const grid = page.getByRole("grid", { name: "Active messages" });
  await grid.getByRole("row").nth(1).click();
  await page.keyboard.press("End");
  await expect(grid.getByRole("row", { selected: true })).toHaveAttribute("data-row-id", "12000");
  // However many messages are loaded, only the rows in view are in the page.
  expect(await grid.getByRole("row").count()).toBeLessThan(120);
  await page.keyboard.press("Home");
  await expect(grid.getByRole("row", { selected: true })).toHaveAttribute("data-row-id", "1");
});

test("from the end, then back to paging with Peek more", async ({ page }) => {
  await open(page, "default", TRACKING, "subscription", "dead-letter");
  await expect(status(page)).toContainText("100 messages");
  await expect(status(page)).toContainText("more available");
  await page.getByRole("radio", { name: "End" }).click();
  await expect(status(page)).toContainText("Last 100 messages");
  // The end has no continuation.
  await expect(status(page).getByRole("button", { name: "Peek more" })).toHaveCount(0);
  await page.getByRole("radio", { name: "Start" }).click();
  await status(page).getByRole("button", { name: "Peek more" }).click();
  await expect(status(page)).toContainText("200 messages");
});

test("a read that fails says why and offers a way on", async ({ page }) => {
  await open(page, "peek-error", INVOICES, "queue", "dead-letter");
  await expect(page.getByRole("heading", { name: "Can't read the dead-letter queue" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(page.locator("[data-row-id]")).toHaveCount(0);
});

test("export all to a file, see the job finish, hear it, and clear it", async ({ page }) => {
  await open(page, "default", INVOICES, "queue", "dead-letter");
  await expect(status(page)).toContainText("37 messages");
  await exportAll(page, /^Export 37 messages…/);

  const jobs = drawer(page).getByRole("list");
  const row = jobs.getByRole("listitem");
  await expect(row.getByText(/^invoice-generate-dead-letter-\d{8}-\d{4}\.json$/)).toBeVisible();
  await expect(row.getByText("37 messages written", { exact: true })).toBeVisible();
  await expect(row.getByText("Done", { exact: true })).toBeVisible();
  await expect(row.getByRole("button", { name: /^Show in (Finder|Explorer|folder)/ })).toBeVisible();
  // Nothing to cancel any more, and the ending was announced to a screen reader.
  await expect(row.getByRole("button", { name: /^Cancel/ })).toHaveCount(0);
  await expect(jobUpdates(page)).toContainText("finished: 37 messages written.");

  // The activity log names the file too.
  await drawer(page).getByRole("tab", { name: "Activity" }).click();
  await expect(drawer(page).getByText(/^Exported 37 messages to invoice-generate-dead-letter-\d{8}-\d{4}\.json$/)).toBeVisible();

  await drawer(page).getByRole("tab", { name: /^Jobs/ }).click();
  await drawer(page).getByRole("button", { name: "Clear finished" }).click();
  await expect(drawer(page).getByText("No jobs", { exact: true })).toBeVisible();
});

test("a slow export can be cancelled, and no file is claimed", async ({ page }) => {
  await open(page, "export-slow", INVOICES, "queue", "dead-letter");
  await expect(status(page)).toContainText("37 messages");
  await exportAll(page, /^Export 37 messages…/);
  const row = drawer(page).getByRole("listitem");
  await expect(row.getByText(/^\d+ of 37$/)).toBeVisible();
  await row.getByRole("button", { name: /^Cancel Export/ }).click();
  await expect(row.getByText("Cancelled", { exact: true })).toBeVisible();
  await expect(row.getByText(/No file was written\./)).toBeVisible();
  await expect(row.getByRole("button", { name: /^Show in/ })).toHaveCount(0);
  await expect(jobUpdates(page)).toContainText("was cancelled.");
});

test("a failing export says what to do and offers to try again", async ({ page }) => {
  await open(page, "export-fails", INVOICES, "queue", "dead-letter");
  await expect(status(page)).toContainText("37 messages");
  await exportAll(page, /^Export 37 messages…/);
  const row = drawer(page).getByRole("listitem");
  await expect(row.getByText("Export failed", { exact: true })).toBeVisible();
  await expect(row.getByText(/connection reset by peer/)).toBeVisible();
  await expect(jobUpdates(page)).toContainText("failed.");
  // The activity log reads "<title> to <file> failed", as the app writes it.
  await drawer(page).getByRole("tab", { name: "Activity" }).click();
  await expect(drawer(page).getByText(/^Export invoice-generate \(dead-letter queue\) to invoice-generate-dead-letter-\d{8}-\d{4}\.json failed$/)).toBeVisible();
  await drawer(page).getByRole("tab", { name: /^Jobs/ }).click();
  await row.getByRole("button", { name: "Try again" }).click();
  await expect(page.getByRole("dialog", { name: "Export messages" })).toBeVisible();
});

test("export selected rows: the sheet counts them, and the job says how many it wrote", async ({ page }) => {
  await open(page, "default", INVOICES, "queue", "dead-letter");
  await expect(status(page)).toContainText("37 messages");
  const grid = page.getByRole("grid", { name: "Dead-letter queue" });
  await grid.getByRole("row").nth(1).click();
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await expect(grid.getByRole("row", { selected: true })).toHaveCount(3);
  await page.getByRole("button", { name: "Export", exact: true }).click();
  await page.getByRole("menuitem", { name: "Export 3 selected…" }).click();
  const sheet = page.getByRole("dialog", { name: "Export messages" });
  await expect(sheet.getByRole("radio", { name: "Selected (3)" })).toBeChecked();
  await sheet.getByRole("button", { name: "Export 3 messages…" }).click();
  await expect(drawer(page).getByRole("listitem").getByText("3 messages written", { exact: true })).toBeVisible();
});

test("sessions: list them, peek one", async ({ page }) => {
  await open(page, "default", "returns-intake", "queue", "sessions");
  const grid = page.getByRole("grid", { name: "Sessions" });
  await expect(grid.getByRole("row", { name: "RMA-1003" })).toBeVisible();
  // Nothing is peeked until a session is chosen; the button follows the selection.
  await expect(page.getByRole("button", { name: "Peek messages" })).toBeDisabled();
  await grid.getByRole("row", { name: "RMA-1003" }).dblclick();
  await expect(page.getByRole("tab", { name: /^Messages/, selected: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Remove session RMA-1003" })).toBeVisible();
  await expect(status(page)).toContainText("4 matching messages");
  // Removing the session condition reads the whole queue again.
  await page.getByRole("button", { name: "Remove session RMA-1003" }).click();
  await expect(page.getByRole("button", { name: "Remove session RMA-1003" })).toHaveCount(0);
  await expect(status(page)).toContainText("48 messages");
});

test("sessions: the Peek messages button peeks the selected session", async ({ page }) => {
  await open(page, "default", "returns-intake", "queue", "sessions");
  const grid = page.getByRole("grid", { name: "Sessions" });
  await grid.getByRole("row", { name: "RMA-1004" }).click();
  await page.getByRole("button", { name: "Peek messages" }).click();
  await expect(page.getByRole("button", { name: "Remove session RMA-1004" })).toBeVisible();
});

test("rules are read-only", async ({ page }) => {
  await open(page, "default", TRACKING, "subscription", "rules");
  const rule = page.getByRole("listitem", { name: "eu-only" });
  await expect(rule).toContainText("SET priority = 'high'");
  await expect(page.getByRole("listitem", { name: "priority" })).toContainText("batch-7");
  await expect(page.getByText("Rules are read-only here.")).toBeVisible();
  await expect(page.getByRole("button", { name: /Edit|Delete|Add rule|Remove/ })).toHaveCount(0);
});

test("overview: counts, copying addresses, and a connection string that is never shown", async ({ page, browserName }) => {
  if (browserName === "chromium") await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await open(page, "default", INVOICES, "queue", "overview");
  const counts = page.getByRole("region", { name: "Counts" });
  await expect(counts.getByText("37", { exact: true })).toHaveClass(/text-accent/);
  await expect(counts.getByText("1,284", { exact: true })).not.toHaveClass(/text-accent/);

  const addresses = page.getByRole("region", { name: "Addresses" });
  await addresses.getByRole("button", { name: "Copy URL", exact: true }).click();
  await expect(appStatus(page).getByText("Copied the URL for invoice-generate")).toBeVisible();
  if (browserName === "chromium") {
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("sb://sb-logistics-prod.servicebus.windows.net/invoice-generate");
  }

  await addresses.getByRole("button", { name: "Copy connection string" }).click();
  await expect(appStatus(page).getByText(/^Copied the connection string for invoice-generate/)).toBeVisible();
  // The string went to the clipboard from the engine: it is in no text on the page, and not in the activity log.
  await drawer(page).getByRole("tab", { name: "Activity" }).click();
  await expect(drawer(page).getByText("Copied the connection string for invoice-generate", { exact: true })).toBeVisible();
  expect(await page.locator("body").innerText()).not.toMatch(/SharedAccessKey|Endpoint=sb:/);
});

test("overview: a connection string scoped to another entity is refused beside the button", async ({ page }) => {
  await open(page, "copy-scoped", INVOICES, "queue", "overview");
  await page.getByRole("button", { name: "Copy connection string" }).click();
  await expect(page.getByRole("alert").filter({ hasText: "Couldn't copy the connection string" })).toBeVisible();
});

test("dashboard: dead letters first, only those, copy as TSV, open an entity", async ({ page, browserName }) => {
  if (browserName === "chromium") await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await open(page, "default");
  await expect(page.getByRole("tree", { name: "Namespace" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+Shift+d");
  const grid = page.getByRole("grid", { name: "Dashboard" });
  // Sorted by dead letters: the entity with the most leads.
  await expect(grid.getByRole("row").nth(1)).toContainText("tracking-sync");

  const only = page.getByRole("radio", { name: /^With dead letters/ });
  await only.click();
  await expect(grid.getByRole("row", { name: "customs-docs" })).toHaveCount(0);
  await expect(grid.getByRole("row", { name: /tracking-sync/ })).toBeVisible();
  await page.getByRole("radio", { name: /^All/ }).click();
  await expect(grid.getByRole("row", { name: "customs-docs" })).toBeVisible();

  await page.getByRole("button", { name: "Copy as TSV" }).click();
  await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  if (browserName === "chromium") {
    const tsv = await page.evaluate(() => navigator.clipboard.readText());
    expect(tsv.split("\n")[0]).toBe("Entity\tTopic\tKind\tStatus\tActive\tDead-letter\tScheduled\tTransfer\tTransfer DLQ");
    expect(tsv.split("\n")[1]).toMatch(/^tracking-sync\tshipment-events\tsubscription\t/);
  }

  await grid.getByRole("row", { name: "label-print" }).dblclick();
  await expect(page.getByRole("heading", { level: 1, name: "label-print" })).toBeVisible();
});

test("a highlight rule marks tree rows with a swatch", async ({ page }) => {
  await open(page, "default");
  const tree = page.getByRole("tree", { name: "Namespace" });
  await expect(tree.getByRole("treeitem", { name: /^invoice-generate/ })).toBeVisible();
  await expect(tree.getByRole("treeitem", { name: /highlighted/ })).toHaveCount(0);
  await page.getByRole("button", { name: "Explorer options" }).click();
  const sheet = page.getByRole("dialog", { name: "Explorer options for Logistics" });
  await sheet.getByRole("button", { name: "Add rule" }).click();
  await sheet.getByLabel("Rule 1 value").fill("^invoice");
  await expect(sheet.getByText("Matches 1 loaded entity")).toBeVisible();
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect(sheet).toHaveCount(0);
  await expect(tree.getByRole("treeitem", { name: /^invoice-generate.*highlighted/ })).toBeVisible();
  await expect(tree.getByRole("treeitem", { name: /highlighted/ })).toHaveCount(1);
});

test("an unsaved connection can be tested, and its secret is never shown", async ({ page }) => {
  await page.goto("/?scenario=empty");
  await page.getByRole("button", { name: "Add connection string" }).click();
  const sheet = page.getByRole("dialog", { name: "Add connection" });
  await sheet.getByLabel("Name", { exact: true }).fill("Orders");
  await sheet.getByLabel("Connection string").fill("Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0");
  await sheet.getByRole("button", { name: "Test connection" }).click();
  await expect(sheet.getByRole("status").getByText(/^Tested /)).toBeVisible();
  // Nothing was saved by testing.
  await expect(page.getByRole("button", { name: /^Connection: Orders/ })).toHaveCount(0);
  await expect(page.getByText("c2VjcmV0")).toHaveCount(0);
});

test("F5 peeks again without reloading the page", async ({ page }) => {
  await open(page, "default", INVOICES, "queue", "dead-letter");
  await expect(status(page)).toContainText("37 messages");
  // Record every state the status line goes through, and mark the document: a reload would lose both.
  await page.evaluate(() => {
    const w = window as unknown as { marker: number; seen: string[] };
    w.marker = 1;
    w.seen = [];
    const el = document.querySelector('[role="status"][aria-label="Peek status"]')!;
    new MutationObserver(() => w.seen.push(el.textContent ?? "")).observe(el, { childList: true, subtree: true, characterData: true });
  });
  await page.keyboard.press("F5");
  await expect.poll(() => page.evaluate(() => (window as unknown as { seen: string[] }).seen.some((t) => t.includes("Reading…")))).toBe(true);
  await expect(status(page)).toContainText("37 messages");
  expect(await page.evaluate(() => (window as unknown as { marker?: number }).marker)).toBe(1);
});

test("a narrow window scrolls the sub-tabs, keeps the active one in view and shows that there are more", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 800 });
  await open(page, "default", "returns-intake", "queue", "listener");
  const views = page.getByRole("tablist", { name: "Entity views" });
  const listener = views.getByRole("tab", { name: "Listener" });
  await expect(listener).toHaveAttribute("aria-selected", "true");
  const geometry = () =>
    views.evaluate((list) => {
      const tab = list.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')!;
      const l = list.getBoundingClientRect();
      const t = tab.getBoundingClientRect();
      return {
        overflows: list.scrollWidth > list.clientWidth,
        scrolled: list.scrollLeft > 0,
        inView: t.left >= l.left - 1 && t.right <= l.right + 1,
        before: list.hasAttribute("data-overflow-start"),
        after: list.hasAttribute("data-overflow-end"),
        windowScrolled: window.scrollX + window.scrollY,
      };
    });
  // Wait for it to settle: the active tab is brought into view as soon as the strip has its size.
  await expect.poll(async () => (await geometry()).inView).toBe(true);
  const atEnd = await geometry();
  expect(atEnd).toMatchObject({ overflows: true, scrolled: true, before: true, after: false, windowScrolled: 0 });

  // Back to the first tab: the strip scrolls back, and the fade moves to the other edge.
  await views.getByRole("tab", { name: "Overview" }).click();
  await expect.poll(async () => (await geometry()).before).toBe(false);
  expect(await geometry()).toMatchObject({ overflows: true, inView: true, after: true });

  // A middle tab is brought in clear of the fade on the edge that has more tabs beyond it (28px), not just into view.
  await views.getByRole("tab", { name: "Scheduled" }).click();
  await expect(views.getByRole("tab", { name: "Scheduled" })).toHaveAttribute("aria-selected", "true");
  const clear = () =>
    views.evaluate((list) => {
      const tab = list.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')!;
      const l = list.getBoundingClientRect();
      const t = tab.getBoundingClientRect();
      const max = list.scrollWidth - list.clientWidth;
      return {
        // Inside the strip by the fade's width (28px, less a pixel for rounding), or at the true start / end where there is no fade.
        left: t.left - l.left >= 27 || list.scrollLeft <= 1,
        right: l.right - t.right >= 27 || list.scrollLeft >= max - 1,
      };
    });
  await expect.poll(clear).toEqual({ left: true, right: true });
  // More tabs on both sides: both fades are on.
  await expect(page.locator('[role="tablist"][aria-label="Entity views"][data-overflow-start][data-overflow-end]')).toHaveCount(1);
});

test("a narrow window keeps every control of the message toolbar inside it", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 800 });
  await open(page, "default", INVOICES, "queue", "dead-letter");
  const toolbar = page.getByRole("toolbar", { name: "Peek options" });
  await expect(status(page)).toContainText("37 messages");
  /** Controls that stick out past the toolbar's edges, or a toolbar that scrolls sideways. */
  const clipped = () =>
    toolbar.evaluate((bar) => {
      const b = bar.getBoundingClientRect();
      const out: string[] = bar.scrollWidth > bar.clientWidth + 1 ? [`toolbar scrolls by ${bar.scrollWidth - bar.clientWidth}px`] : [];
      for (const el of bar.querySelectorAll<HTMLElement>("button, input, [role=combobox], [role=radio]")) {
        const r = el.getBoundingClientRect();
        if (r.width > 0 && (r.left < b.left - 0.5 || r.right > b.right + 0.5)) out.push(`${el.getAttribute("aria-label") ?? el.textContent}: ${Math.round(r.right - b.right)}px past the edge`);
      }
      return out;
    });
  expect(await clipped()).toEqual([]);
  // The controls keep their names when their labels give way to icons.
  for (const name of ["Filter", "Columns", "Export", "Peek again"]) await expect(toolbar.getByRole("button", { name })).toBeVisible();
  // A sequence number field and an applied filter make the row longer still.
  await toolbar.getByRole("radio", { name: "Sequence" }).click();
  await expect(toolbar.getByRole("textbox", { name: "From sequence number" })).toBeVisible();
  expect(await clipped()).toEqual([]);
});
