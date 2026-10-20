import { expect, test, type Page } from "@playwright/test";

const open = (page: Page, scenario: string, seed?: Record<string, unknown>) =>
  page.goto(`/?scenario=${scenario}&autoconnect=p-logistics${seed ? `&seed=${encodeURIComponent(JSON.stringify(seed))}` : ""}`);

/** Runs a palette command by its label. */
async function palette(page: Page, label: string) {
  await page.keyboard.press("ControlOrMeta+k");
  await page.getByPlaceholder("Go to entity or command").fill(label.replace("…", ""));
  await page.getByRole("dialog", { name: "Command palette" }).getByRole("option", { name: label }).click();
}

/** Submits an operation sheet, confirms it (off production: one click) and closes it on its report. */
async function confirmAndClose(page: Page, sheetName: string, action: string) {
  const sheet = page.getByRole("dialog", { name: sheetName });
  await sheet.getByRole("button", { name: action }).click();
  await sheet.getByRole("button", { name: "Confirm" }).click();
  await expect(sheet.getByRole("button", { name: "Close", exact: true }).last()).toBeVisible();
  await sheet.getByRole("button", { name: "Close", exact: true }).last().click();
}

test("create, edit, rename and delete a queue; the tab follows", async ({ page }) => {
  await open(page, "operate-ok");
  await expect(page.getByRole("tree", { name: "Namespace" })).toBeVisible();

  await palette(page, "New queue…");
  await page.getByRole("dialog", { name: "New queue" }).getByRole("textbox", { name: "Name" }).fill("orders");
  await confirmAndClose(page, "New queue", "Create queue");
  const tree = page.getByRole("tree", { name: "Namespace" });
  await expect(tree.getByRole("treeitem", { name: /^orders/ })).toBeVisible();

  await tree.getByRole("treeitem", { name: /^orders/ }).dblclick();
  await expect(page.getByRole("heading", { level: 1, name: "orders" })).toBeVisible();
  await page.getByRole("button", { name: "Edit settings…" }).click();
  const edit = page.getByRole("dialog", { name: "Settings of orders" });
  await edit.getByRole("textbox", { name: "Max delivery count" }).fill("3");
  await confirmAndClose(page, "Settings of orders", "Save 1 change");

  await palette(page, "Rename orders…");
  await page.getByRole("dialog", { name: "Rename orders" }).getByRole("textbox", { name: "New name" }).fill("orders-2");
  await confirmAndClose(page, "Rename orders", "Rename queue");
  await expect(page.getByRole("heading", { level: 1, name: "orders-2" })).toBeVisible();

  await palette(page, "Delete orders-2…");
  await confirmAndClose(page, "Delete orders-2", "Delete queue");
  await expect(page.getByText("This entity wasn't found")).toBeVisible();
});

test("the listener watches new messages until stopped", async ({ page }) => {
  await open(page, "operate-ok", {
    "shell:p-logistics": {
      tabs: [{ id: "label-print", kind: "queue", subTab: "listener" }],
      activeTab: "label-print",
      expanded: [],
      selected: null,
      layout: { treeSize: 18, drawerOpen: false, drawerSize: 22, drawerTab: "jobs" },
    },
  });
  const panel = page.getByRole("region", { name: "Listener" });
  await panel.getByRole("button", { name: "Start watching" }).click();
  const grid = panel.getByRole("grid", { name: "Messages received" });
  await expect(grid.getByRole("row")).not.toHaveCount(1);
  await expect(panel.getByRole("img", { name: /^Messages per second/ })).toBeVisible();
  await panel.getByRole("button", { name: "Stop" }).click();
  await expect(panel.getByRole("button", { name: "New listener" })).toBeVisible();
  await expect(grid.getByRole("row").nth(1)).toBeVisible();
});
