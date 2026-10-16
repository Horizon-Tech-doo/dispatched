import { expect, test, type Page } from "@playwright/test";

const go = (page: Page, query: string) => page.goto(`/?${query}`);
const tree = (page: Page) => page.getByRole("tree", { name: "Namespace" });
const logistics = "scenario=default&autoconnect=p-logistics";
const activity = (page: Page) => page.getByRole("region", { name: "Jobs and activity" });

test("browse: counts in the tree, open a subscription, read its views", async ({ page }) => {
  await go(page, logistics);
  const t = tree(page);
  const invoice = t.getByRole("treeitem", { name: "invoice-generate" });
  await expect(invoice).toBeVisible();
  // Dead letters are the one count that is amber; the others are muted.
  await expect(invoice.getByText("37", { exact: true })).toHaveClass(/text-accent/);
  await expect(invoice.getByText("1,284", { exact: true })).not.toHaveClass(/text-accent/);
  await t.getByRole("treeitem", { name: "shipment-events", exact: true }).click();
  await page.keyboard.press("ArrowRight");
  await t.getByRole("treeitem", { name: "tracking-sync" }).click();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tablist", { name: "Open entities" }).getByRole("tab", { name: /tracking-sync/ })).toHaveAttribute("aria-selected", "true");
  const views = page.getByRole("tablist", { name: "Entity views" });
  await expect(page.getByRole("heading", { level: 1, name: "tracking-sync" })).toBeVisible();
  // 3,417 is the subscription's dead-letter count, so it is amber.
  await expect(views.getByText("3,417")).toHaveClass(/text-accent/);
  await views.getByRole("tab", { name: /Dead-letter/ }).click();
  await expect(page.getByRole("grid", { name: "Dead-letter queue" })).toBeVisible();
  await expect(page.getByRole("status", { name: "Peek status" })).toContainText("100 messages");
  await expect(page.getByRole("banner").getByText("PROD", { exact: true })).toBeVisible();
  await expect(page.getByRole("contentinfo").getByText("Peek only, nothing locked")).toBeVisible();
});

test("palette opens an entity", async ({ page }) => {
  await go(page, logistics);
  await expect(tree(page).getByRole("treeitem", { name: "label-print" })).toBeVisible();
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByPlaceholder("Go to entity or command").fill("label-print");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true");
  await expect(page.getByRole("dialog", { name: "Command palette" })).toHaveCount(0);
});

test("first connection from an empty start", async ({ page }) => {
  await go(page, "scenario=empty");
  await page.getByRole("button", { name: "Add connection string" }).click();
  const sheet = page.getByRole("dialog", { name: "Add connection" });
  await sheet.getByLabel("Name", { exact: true }).fill("Orders");
  await sheet.getByLabel("Connection string").fill("Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0");
  await sheet.getByRole("button", { name: "Save" }).click();
  await expect(page.getByRole("button", { name: /^Connection: Orders, orders/ })).toBeVisible();
  await expect(sheet).toHaveCount(0);
  await expect(tree(page)).toBeVisible();
  // The secret goes to the keychain and never back to the page.
  await expect(page.getByText("c2VjcmV0")).toHaveCount(0);
});

test("limited listing explains itself and shows pinned entities", async ({ page }) => {
  await go(page, "scenario=forbidden&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Showing pinned entities only" })).toBeVisible();
  await expect(page.getByText(/This identity can't list entities in .*\. Grant it Manage rights/)).toBeVisible();
  await expect(tree(page).getByRole("treeitem", { name: "orders" })).toBeVisible();
});

test("missing secret shows the fix", async ({ page }) => {
  await go(page, "scenario=missing-secret&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Couldn't connect to Logistics" })).toBeVisible();
  await expect(page.getByText("The connection string for Logistics isn't in the system keychain. Edit the connection and paste it again.")).toBeVisible();
  await page.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByRole("dialog", { name: "Edit Logistics", exact: true })).toBeVisible();
});

test("listing errors offer to try again and land in the activity log", async ({ page }) => {
  await go(page, "scenario=network&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Can't list this namespace" })).toBeVisible();
  await expect(page.getByText(/The connection to .* was interrupted/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
  await expect(activity(page).getByText("Listing the namespace failed").first()).toBeVisible();
  await expect(activity(page).getByText(/connection reset by peer/).first()).toBeVisible();
});

test("theme follows settings", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await go(page, logistics);
  await expect(tree(page)).toBeVisible();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.keyboard.press("ControlOrMeta+,");
  await page.getByRole("dialog", { name: "Settings" }).getByRole("radio", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("2,000 queues stay navigable from the keyboard", async ({ page }) => {
  await go(page, "scenario=many&autoconnect=p-logistics");
  const t = tree(page);
  await t.getByRole("treeitem", { name: "queue-0000" }).click();
  await page.keyboard.press("End");
  await expect(t.getByRole("treeitem", { name: "stock-levels" })).toBeVisible();
  expect(await t.getByRole("treeitem").count()).toBeLessThan(120);
  await page.keyboard.press("Home");
  await expect(t.getByRole("treeitem", { name: "queue-0000" })).toBeVisible();
});

test("drawer and tabs follow the keyboard", async ({ page }) => {
  await go(page, logistics);
  await tree(page).getByRole("treeitem", { name: "label-print" }).dblclick();
  const tab = page.getByRole("tablist", { name: "Open entities" }).getByRole("tab", { name: /label-print/ });
  await expect(tab).toBeVisible();
  await expect(activity(page)).toBeVisible();
  await page.keyboard.press("ControlOrMeta+j");
  await expect(activity(page)).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+j");
  await expect(activity(page)).toBeVisible();
  await tree(page).focus();
  await page.keyboard.press("ControlOrMeta+w");
  await expect(tab).toHaveCount(0);
});

/** Which tab in `list` draws the marker, read in one go right after a click, with nothing waited for: the
 *  marker must move in the same frame. Underline tabs draw it as the trigger's bottom border; the entity
 *  strip draws it as the top border of the tab's wrapper (so the close button shares it). */
const markerState = (page: Page, list: string) =>
  page.getByRole("tablist", { name: list }).evaluate((el) => {
    const fg = getComputedStyle(document.body).color;
    const tabs = [...el.querySelectorAll<HTMLElement>('[role="tab"]')].map((t) => {
      const strip = t.parentElement !== el;
      const cs = getComputedStyle(strip ? t.parentElement! : t);
      const marker = strip ? cs.borderTopColor : cs.borderBottomColor;
      const box = t.getBoundingClientRect();
      return { name: t.textContent?.trim() ?? "", selected: t.getAttribute("aria-selected") === "true", marked: marker === fg, left: box.left, right: box.right };
    });
    return { tabs, running: document.getAnimations().length };
  });

test("the tab marker moves with the click, also with reduced motion", async ({ page }) => {
  // Reduced motion is where WebKit used to keep painting the old tab: a global 0.01ms transition
  // started on every style change and the marker stayed behind until the next frame.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await go(page, logistics);
  await tree(page).getByRole("treeitem", { name: "label-print" }).dblclick();
  await tree(page).getByRole("treeitem", { name: "invoice-generate" }).dblclick();
  const expectMarkerOn = async (list: string, name: RegExp) => {
    const { tabs, running } = await markerState(page, list);
    expect(running, "nothing animates under reduced motion").toBe(0);
    const marked = tabs.filter((t) => t.marked);
    expect(marked.map((t) => t.name)).toEqual([expect.stringMatching(name)]);
    expect(marked[0].selected).toBe(true);
  };

  const drawer = activity(page);
  await expectMarkerOn("Drawer", /^Activity/);
  await drawer.getByRole("tab", { name: /^Jobs/ }).click();
  await expectMarkerOn("Drawer", /^Jobs/);
  await expect(drawer.getByText("Exports and bulk operations show their progress here")).toBeVisible();
  // The marker sits under the Jobs label, not under Activity.
  const jobsBox = (await drawer.getByRole("tab", { name: /^Jobs/ }).boundingBox())!;
  const { tabs } = await markerState(page, "Drawer");
  const marked = tabs.find((t) => t.marked)!;
  expect(marked.left).toBeCloseTo(jobsBox.x, 0);
  expect(marked.right).toBeCloseTo(jobsBox.x + jobsBox.width, 0);
  await drawer.getByRole("tab", { name: "Activity" }).click();
  await expectMarkerOn("Drawer", /^Activity/);

  await expectMarkerOn("Open entities", /^invoice-generate/);
  await page.getByRole("tablist", { name: "Open entities" }).getByRole("tab", { name: /label-print/ }).click();
  await expectMarkerOn("Open entities", /^label-print/);

  await expectMarkerOn("Entity views", /^Overview/);
  await page.getByRole("tablist", { name: "Entity views" }).getByRole("tab", { name: /^Dead-letter/ }).click();
  await expectMarkerOn("Entity views", /^Dead-letter/);
});

test("sub-tab counts take only their own width, so short counts leave no gap before the next tab", async ({ page }) => {
  await go(page, logistics);
  await tree(page).getByRole("treeitem", { name: "invoice-generate" }).dblclick();
  const views = page.getByRole("tablist", { name: "Entity views" });
  const count = views.getByText("1,284", { exact: true });
  await expect(count).toBeVisible();
  // The room between a count and the end of its tab is the tab's padding, whatever the count's width.
  const gaps = await count.evaluate((el) => {
    const tab = el.closest('[role="tab"]')!;
    const original = el.textContent;
    const out = ["0", "37", "2,906", "99,999"].map((t) => {
      el.textContent = t;
      return Math.round(tab.getBoundingClientRect().right - el.getBoundingClientRect().right);
    });
    el.textContent = original;
    return out;
  });
  expect(new Set(gaps).size).toBe(1);
  expect(gaps[0]).toBeLessThanOrEqual(12);
});

test("filter bar: Apply, Clear filter and the body segments take one click while the property field has focus", async ({ page }) => {
  await go(page, logistics);
  await tree(page).getByRole("treeitem", { name: "invoice-generate" }).dblclick();
  await page.getByRole("tablist", { name: "Entity views" }).getByRole("tab", { name: /^Dead-letter/ }).click();
  await expect(page.getByRole("status", { name: "Peek status" })).toContainText("37 messages");
  await page.getByRole("button", { name: "Filter", exact: true }).click();
  const bar = page.getByRole("search", { name: "Message filter" });
  const properties = bar.getByLabel("Properties");

  // Focus in Properties shows the syntax help. Pressing a control moves focus and hides it: the control must not
  // move between the pointer going down and coming up, or the click is lost.
  await properties.click();
  await properties.fill("attempt >= 2");
  await expect(bar.getByText("sys.MessageId")).toBeVisible();
  await bar.getByRole("button", { name: "Apply" }).click();
  await expect(page.getByRole("button", { name: "Filter 1", exact: true })).toBeVisible();

  await properties.click();
  await bar.getByRole("radio", { name: "Regex" }).click();
  await expect(bar.getByRole("radio", { name: "Regex" })).toBeChecked();

  await properties.click();
  await bar.getByRole("button", { name: "Clear filter" }).click();
  await expect(page.getByRole("button", { name: "Filter", exact: true })).toBeVisible();
  await expect(properties).toHaveValue("");
});

test("message grid columns keep their minimum widths beside the detail pane, and scroll sideways only when they cannot fit", async ({ page }) => {
  const tracking = "shipment-events/subscriptions/tracking-sync";
  const seed = encodeURIComponent(
    JSON.stringify({
      "shell:p-logistics": {
        tabs: [{ id: tracking, kind: "subscription", subTab: "dead-letter" }], activeTab: tracking, expanded: [], selected: null,
        layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" },
      },
    }),
  );
  await go(page, `${logistics}&seed=${seed}`);
  await expect(page.getByRole("status", { name: "Peek status" })).toContainText("100 messages");
  const grid = page.getByRole("grid");
  const widthOf = async (name: string) => (await grid.getByRole("columnheader", { name }).boundingBox())!.width;
  const overflows = () => grid.evaluate((el) => el.scrollWidth > el.clientWidth + 1);

  // The whole width: the default dead-letter columns fit at their preferred widths, nothing scrolls.
  expect(await overflows()).toBe(false);
  // Beside the pane the panel is narrower than the columns' minimums: each keeps its own minimum and the grid scrolls.
  await page.locator("[data-row-id]").first().click();
  await grid.press("Enter");
  await expect(page.getByRole("region", { name: /^Message \d+$/ })).toBeVisible();
  expect(await overflows()).toBe(true);
  expect(await widthOf("Subject")).toBeGreaterThanOrEqual(109);
  expect(await widthOf("Message ID")).toBeGreaterThanOrEqual(119);
  expect(await widthOf("Reason")).toBeGreaterThanOrEqual(149);
  expect(await widthOf("Description")).toBeGreaterThanOrEqual(199);
});
