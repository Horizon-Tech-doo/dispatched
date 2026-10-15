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
  await expect(page.getByRole("heading", { name: "Dead-letter isn't available yet" })).toBeVisible();
  await expect(page.getByRole("banner").getByText("PROD", { exact: true })).toBeVisible();
  await expect(page.getByText("Peek only, nothing locked")).toBeVisible();
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
  await expect(page.getByText(/This identity can't list entities in .*, so only pinned entities are shown/)).toBeVisible();
  await expect(tree(page).getByRole("treeitem", { name: "orders" })).toBeVisible();
});

test("missing secret shows the fix", async ({ page }) => {
  await go(page, "scenario=missing-secret&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Couldn't connect to Logistics" })).toBeVisible();
  await expect(page.getByText("The connection string for Logistics isn't in the system keychain. Edit the connection and paste it again.")).toBeVisible();
  await page.getByRole("button", { name: "Edit connection" }).click();
  await expect(page.getByRole("dialog", { name: "Edit Logistics", exact: true })).toBeVisible();
});

test("listing errors offer a retry and land in the activity log", async ({ page }) => {
  await go(page, "scenario=network&autoconnect=p-logistics");
  await expect(page.getByRole("heading", { name: "Can't list this namespace" })).toBeVisible();
  await expect(page.getByText(/The connection to .* was interrupted/)).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry" })).toBeVisible();
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
