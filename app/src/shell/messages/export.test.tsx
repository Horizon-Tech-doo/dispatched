import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, setTransport, type IpcTransport } from "@/engine";
import { activityStore } from "../drawer/activity";
import { renderShell } from "../test-utils";
import { setViewState, viewKey, type ViewState } from "./viewStore";

const seed = {
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id: "invoice-generate", kind: "queue", subTab: "dead-letter" }], activeTab: "invoice-generate", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
};

const TRACKING = "shipment-events/subscriptions/tracking-sync";
const trackingSeed = {
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id: TRACKING, kind: "subscription", subTab: "dead-letter" }], activeTab: TRACKING, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
};

type Request = Record<string, unknown> & { scope: { kind: string; sequences?: number[]; filter?: Record<string, unknown> } };
const startOf = (mock: { calls: { cmd: string; args?: unknown }[] }) =>
  (mock.calls.find((c) => c.cmd === "export_start")!.args as { profileId: string; request: Request });
/** In jsdom the resizable panels swallow the pointer-down Radix opens menus on: open the menu by keyboard. */
const openExportMenu = async () => {
  act(() => screen.getByRole("button", { name: "Export" }).focus());
  await userEvent.keyboard("{Enter}");
};
const sheetOf = () => screen.getByRole("dialog", { name: "Export messages" });

/** The mock, with `edit` able to refuse a command before the mock sees it. */
function around(mock: IpcTransport, edit: (cmd: string, args: Record<string, unknown> | undefined, answer: () => Promise<unknown>) => Promise<unknown>) {
  setTransport({ listen: mock.listen, invoke: <T,>(cmd: string, args?: Record<string, unknown>) => edit(cmd, args, () => mock.invoke(cmd, args)) as Promise<T> });
}

describe("export", () => {
  it("exports the view as NDJSON through the save dialog and shows the job", async () => {
    const { mock } = renderShell("default", seed);
    const grid = await screen.findByRole("grid");
    await within(grid).findAllByText("MaxDeliveryCountExceeded");
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    const sheet = sheetOf();
    await userEvent.click(within(sheet).getByRole("radio", { name: "NDJSON" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file and export" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "export_start")).toBe(true));
    expect(mock.calls.find((c) => c.cmd === "pick_export_path")!.args).toEqual({ entity: "invoice-generate", view: "dead-letter", format: "ndjson" });
    const { profileId, request } = startOf(mock);
    expect(profileId).toBe("p-logistics");
    expect(request).toMatchObject({ entity: "invoice-generate", view: "dead-letter", format: "ndjson", scope: { kind: "all", filter: {} }, total_hint: 37 });
    // The webview never sends a path or an overwrite flag; the pick is the engine's one-time id.
    expect(typeof request.pick).toBe("string");
    expect(request).not.toHaveProperty("path");
    expect(request).not.toHaveProperty("overwrite");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Export messages" })).toBeNull());
    const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
    expect(await within(jobs).findByText(/Exported 37 messages to/)).toBeInTheDocument();
    expect(within(jobs).getByRole("tab", { name: "Jobs" })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(within(jobs).getByRole("button", { name: "Show in folder" }));
    expect(mock.calls.some((c) => c.cmd === "job_reveal")).toBe(true);
    // An export is something the user did: the activity log names it, and it is not a failure.
    expect(activityStore.all().some((e) => /^Exported 37 messages to /.test(e.action) && e.error === null)).toBe(true);
  });

  it("exports only the selected messages", async () => {
    const { mock } = renderShell("default", seed);
    const grid = await screen.findByRole("grid");
    await within(grid).findAllByText("MaxDeliveryCountExceeded");
    // The first three rows, by keyboard (the grid's pointer handling is out of reach in jsdom).
    act(() => grid.focus());
    await userEvent.keyboard("{Home}{Shift>}{ArrowDown}{ArrowDown}{/Shift}");
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: /Export 3 selected/ }));
    const sheet = sheetOf();
    expect(within(sheet).getByRole("radio", { name: "Selected (3)" })).toBeChecked();
    expect(within(sheet).getByText(/3 selected messages of invoice-generate, read again when the export runs/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file and export" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "export_start")).toBe(true));
    const { request } = startOf(mock);
    expect(request.scope.kind).toBe("selected");
    expect(request.scope.sequences).toHaveLength(3);
    expect(request.total_hint ?? null).toBeNull();
  });

  it("offers no selected export while nothing is selected", async () => {
    renderShell("default", seed);
    await within(await screen.findByRole("grid")).findAllByText("MaxDeliveryCountExceeded");
    await openExportMenu();
    expect(screen.getByRole("menuitem", { name: "Export all…" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /selected/ })).toBeNull();
  });

  it("exports all with the view's current filter and says so", async () => {
    const defaults: ViewState = { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: false, sorting: null, detailOpen: false };
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid");
    act(() => setViewState(viewKey("p-logistics", "invoice-generate", "dead-letter"), defaults, { filter: { body_contains: "SHP" } }));
    await waitFor(() => expect(screen.getByRole("status", { name: "Peek status" })).toHaveTextContent(/matching message|scanned/));
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    const sheet = sheetOf();
    expect(within(sheet).getByText(/that matches the current filter/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file and export" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "export_start")).toBe(true));
    const { request } = startOf(mock);
    expect(request.scope).toMatchObject({ kind: "all", filter: { body_contains: "SHP" } });
    // The count of the whole view would overstate a filtered export.
    expect(request.total_hint ?? null).toBeNull();
  });

  it("cancels a running export from the Jobs tab", async () => {
    const { mock } = renderShell("export-slow", seed);
    await within(await screen.findByRole("grid")).findAllByText("MaxDeliveryCountExceeded");
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    await userEvent.click(within(sheetOf()).getByRole("button", { name: "Choose file and export" }));
    const jobs = await screen.findByRole("region", { name: "Jobs and activity" });
    await userEvent.click(await within(jobs).findByRole("button", { name: /^Cancel Export invoice-generate/ }));
    expect(mock.calls.some((c) => c.cmd === "job_cancel")).toBe(true);
    expect(await within(jobs).findByText("Cancelled")).toBeInTheDocument();
    expect(within(jobs).queryByRole("button", { name: "Show in folder" })).toBeNull();
  });

  it("does nothing when the save dialog is cancelled", async () => {
    const { mock } = renderShell("default", seed);
    await within(await screen.findByRole("grid")).findAllByText("MaxDeliveryCountExceeded");
    around(mock, async (cmd, _args, answer) => (cmd === "pick_export_path" ? null : answer()));
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    await userEvent.click(within(sheetOf()).getByRole("button", { name: "Choose file and export" }));
    // The sheet stays, ready for another try, and no export started.
    expect(await within(sheetOf()).findByRole("button", { name: "Choose file and export" })).toBeEnabled();
    expect(mock.calls.some((c) => c.cmd === "export_start")).toBe(false);
  });

  it("keeps the picked file when the request is refused, so a retry does not ask again", async () => {
    const { mock } = renderShell("default", seed);
    await within(await screen.findByRole("grid")).findAllByText("MaxDeliveryCountExceeded");
    let refused = false;
    const sent: string[] = [];
    around(mock, async (cmd, args, answer) => {
      if (cmd === "export_start") {
        sent.push((args as { request: { pick: string } }).request.pick);
        if (!refused) {
          refused = true;
          throw new EngineError("invalid", "Property filter: unknown name `x`");
        }
      }
      return answer();
    });
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    await userEvent.click(within(sheetOf()).getByRole("button", { name: "Choose file and export" }));
    const sheet = sheetOf();
    const problem = await within(sheet).findByRole("alert");
    expect(within(problem).getByText("Couldn't start the export")).toBeInTheDocument();
    expect(within(problem).getByText(/unknown name/)).toBeInTheDocument();
    expect(within(sheet).getByText(/^invoice-generate-dead-letter-.*\.json$/)).toBeInTheDocument();
    expect(within(sheet).getByText("/Users/ops/Downloads")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Export" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Export messages" })).toBeNull());
    expect(mock.calls.filter((c) => c.cmd === "pick_export_path")).toHaveLength(1);
    expect(sent).toHaveLength(2);
    expect(sent[1]).toBe(sent[0]);
  });

  it("opens the save dialog again when the engine no longer has the pick", async () => {
    const { mock } = renderShell("default", seed);
    await within(await screen.findByRole("grid")).findAllByText("MaxDeliveryCountExceeded");
    let spent = false;
    around(mock, async (cmd, _args, answer) => {
      if (cmd === "export_start" && !spent) {
        spent = true;
        throw new EngineError("invalid", "Choose where to save the export.");
      }
      return answer();
    });
    await openExportMenu();
    await userEvent.click(screen.getByRole("menuitem", { name: "Export all…" }));
    await userEvent.click(within(sheetOf()).getByRole("button", { name: "Choose file and export" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Export messages" })).toBeNull());
    expect(mock.calls.filter((c) => c.cmd === "pick_export_path")).toHaveLength(2);
  });

  it("exports a cut message from the detail pane's note", async () => {
    const { mock } = renderShell("default", trackingSeed);
    const grid = await screen.findByRole("grid");
    await within(grid).findAllByRole("row");
    await userEvent.click(screen.getByRole("radio", { name: "End" }));
    await waitFor(() => expect(screen.getByRole("status", { name: "Peek status" })).toHaveTextContent("Last 100 messages"));
    grid.focus();
    await userEvent.keyboard("{End}{Enter}");
    await userEvent.click(await screen.findByRole("button", { name: "Export message" }));
    const sheet = sheetOf();
    expect(within(sheet).getByRole("radio", { name: "Selected (1)" })).toBeChecked();
    expect(within(sheet).getByText(/1 selected message of /)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file and export" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "export_start")).toBe(true));
    expect(startOf(mock).request.scope).toEqual({ kind: "selected", sequences: [48213] });
  });
});
