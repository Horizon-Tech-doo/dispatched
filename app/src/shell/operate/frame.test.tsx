import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { commands, onJob, setTransport, type ConfirmRequest, type JobEvent } from "@/engine";
import { useShell } from "../context";
import { jobStore } from "../drawer/activity";
import { finished, itemEvent, report, started } from "../drawer/jobEvents";
import { renderShell } from "../test-utils";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

/** Delete two dead letters; Close unmounts the sheet (as the real sheets do). */
function Harness() {
  const { conn } = useShell();
  const [open, setOpen] = useState(true);
  const op = useOperation();
  if (!open) return null;
  return (
    <OperationSheet
      title="Delete messages"
      op={op}
      action="Delete 2 messages"
      onSubmit={() => op.start(() => commands.opDelete(conn.profileId!, { entity: "invoice-generate", view: "dead-letter", selection: { type: "sequences", sequences: ["41000", "41001"] } }))}
      onClose={() => setOpen(false)}
    >
      <p>The form</p>
    </OperationSheet>
  );
}

const ask = (p: Partial<ConfirmRequest> = {}): ConfirmRequest => ({
  id: "c1", job_id: "j1", verb: "delete", target: "invoice-generate", summary: "Delete 2 messages from invoice-generate/$DeadLetterQueue on Logistics",
  item_count: "2", reversible: false, production: false, step: 1, ...p,
});

const connectedTo = (mock: { calls: { cmd: string }[] }) => waitFor(() => expect(mock.calls.some((c) => c.cmd === "list_entities")).toBe(true));

/** The harness whose op_delete starts job `j1` and runs nothing (the test drives its events), or fails with `refuse`. */
async function scripted(refuse?: { kind: string; message: string }, jobReport?: () => Promise<unknown>) {
  const { mock, ...rest } = renderShell("default", { last_profile: "p-logistics" }, <Harness />);
  await connectedTo(mock);
  setTransport({
    listen: mock.listen,
    invoke: <T,>(cmd: string, args?: Record<string, unknown>) =>
      (cmd === "op_delete"
        ? refuse ? Promise.reject(refuse) : Promise.resolve({ id: "j1" })
        : cmd === "job_report" && jobReport ? jobReport() : mock.invoke(cmd, args)) as Promise<T>,
  });
  const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
  return { mock, sheet, ...rest };
}

async function startJob(sheet: HTMLElement) {
  await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
  act(() => jobStore.apply(started("j1", "Delete 2 from invoice-generate/$DeadLetterQueue", null, null, "delete")));
}

describe("operation frame fixes", () => {
  it("moves focus off the action when a confirmation arrives, and announces the stage", async () => {
    const { sheet } = await scripted();
    const action = within(sheet).getByRole("button", { name: "Delete 2 messages" });
    await startJob(sheet);
    act(() => jobStore.confirm(ask()));
    const confirm = within(sheet).getByRole("button", { name: "Delete from invoice-generate" });
    expect(confirm).not.toBe(action);
    await waitFor(() => expect(document.activeElement).toBe(within(sheet).getByRole("button", { name: "Cancel" })));
    const live = sheet.querySelector('[aria-live="polite"]')!;
    expect(live).toHaveTextContent("Delete 2 messages from invoice-generate");
  });

  it("closing on a later confirmation, after the job has done work, cancels the job (purge's second entity)", async () => {
    const { mock, sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.confirm(ask({ verb: "purge", target: "label-print" })));
    await userEvent.click(within(sheet).getByRole("button", { name: "Purge label-print" }));
    act(() => jobStore.apply(itemEvent("j1", { entity: "label-print", outcome: "succeeded" })));
    act(() => jobStore.confirm(ask({ id: "c2", verb: "purge", target: "customs-docs", step: 2 })));
    await userEvent.click(within(sheet).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Delete messages" })).toBeNull();
    expect(mock.calls.some((c) => c.cmd === "job_cancel" && (c.args as { id: string }).id === "j1")).toBe(true);
  });

  it("before any confirmation is given the running footer says Cancel and close; after, Show in Jobs keeps the job", async () => {
    const { mock, sheet } = await scripted();
    await startJob(sheet);
    expect(within(sheet).queryByRole("button", { name: "Show in Jobs" })).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Cancel and close" })).toBeInTheDocument();
    act(() => jobStore.confirm(ask()));
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete from invoice-generate" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Show in Jobs" }));
    expect(screen.queryByRole("dialog", { name: "Delete messages" })).toBeNull();
    expect(mock.calls.some((c) => c.cmd === "job_cancel")).toBe(false);
    // The drawer can answer a later question of a job whose sheet is gone.
    act(() => jobStore.confirm(ask({ id: "c2", step: 2, target: "label-print", verb: "move" })));
    const drawer = screen.getByRole("region", { name: "Jobs and activity" });
    await userEvent.click(within(drawer).getByRole("button", { name: /^Confirm…/ }));
    const again = await screen.findByRole("dialog", { name: "Delete 2 from invoice-generate/$DeadLetterQueue" });
    expect(within(again).getByText("Confirmation 2")).toBeInTheDocument();
  });

  it("a refused start puts the named problem first, with access advice for a missing right", async () => {
    const { sheet } = await scripted({ kind: "forbidden", message: "access denied (needs Send): Unauthorized access. 'Send' claim(s) are required" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
    const alert = await within(sheet).findByRole("alert");
    expect(alert).toHaveTextContent("Couldn't delete messages");
    expect(alert).toHaveTextContent("This key has no Send right");
    expect(alert.compareDocumentPosition(within(sheet).getByText("The form")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("a failed job offers Edit and try again, back to the form", async () => {
    const { sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.apply(finished("j1", { error: { kind: "network", message: "network error: connection reset by peer" } })));
    expect(await within(sheet).findByText(/Check the connection, then try again/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Edit and try again" }));
    expect(within(sheet).getByText("The form")).toBeInTheDocument();
  });

  it("a report that fails to load says so, never that everything was done", async () => {
    const { sheet } = await scripted(undefined, () => Promise.reject({ kind: "not_found", message: "That job has no report." }));
    await startJob(sheet);
    act(() => jobStore.apply(finished("j1", { report: report({ outcome: "partly-failed", items: 3, succeeded: 2, failed: 1, incomplete: 1, summary_line: "Deleted 2 of 3" }) })));
    expect(await within(sheet).findByText("Couldn't load the report")).toBeInTheDocument();
    expect(within(sheet).queryByText("Everything was done.")).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("a job cancelled mid-run with its partial report shows Cancelled, not Done, in the sheet and the drawer", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, <Harness />);
    await connectedTo(mock);
    const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
    // The mock emits finished with error `cancelled` and a non-null report once the job is cancelled after its first item.
    const events: JobEvent[] = [];
    const off = await onJob((e) => {
      events.push(e);
      if (e.type === "item" && events.filter((x) => x.type === "item").length === 1) void commands.jobCancel(e.id);
    });
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
    await userEvent.click(await within(sheet).findByRole("button", { name: "Delete from invoice-generate" }));
    await waitFor(() => expect(events.some((e) => e.type === "finished")).toBe(true));
    const fin = events.find((e): e is Extract<JobEvent, { type: "finished" }> => e.type === "finished")!;
    expect(fin.error?.kind).toBe("cancelled");
    expect(fin.report).not.toBeNull();
    expect(await within(sheet).findByText("Cancelled after 1 deleted.")).toBeInTheDocument();
    await userEvent.click(within(sheet).getAllByRole("button", { name: "Close" }).at(-1)!);
    const drawer = screen.getByRole("region", { name: "Jobs and activity" });
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    const row = within(drawer).getByRole("listitem");
    expect(within(row).getByText("Cancelled")).toBeInTheDocument();
    expect(within(row).queryByText("Done")).toBeNull();
    expect(within(row).getByRole("button", { name: /View report/ })).toBeInTheDocument();
    off();
  });

  it("a report with outcome cancelled and no error still reads Cancelled", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    act(() => jobStore.apply(started("j9", "Delete 3 from orders", null, null, "delete")));
    act(() => jobStore.apply(finished("j9", { report: report({ outcome: "cancelled", cancelled: true, items: 1, succeeded: 1, incomplete: 0 }) })));
    const row = within(drawer).getByRole("listitem");
    expect(within(row).getByText("Cancelled")).toBeInTheDocument();
    expect(within(row).getByText("Cancelled after 1 deleted.")).toBeInTheDocument();
  });

  it("a failed operate row in the drawer names the next step", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const drawer = await screen.findByRole("region", { name: "Jobs and activity" });
    act(() => within(drawer).getByRole("tab", { name: /Jobs/ }).focus());
    act(() => jobStore.apply(started("j9", "Delete 3 from orders", null, null, "delete")));
    act(() => jobStore.apply(finished("j9", { error: { kind: "network", message: "network error" } })));
    expect(within(drawer).getByText("Check the connection, then try again.")).toBeInTheDocument();
  });
});
