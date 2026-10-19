import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { commands, setTransport, type ConfirmRequest, type ItemView, type ReportPage } from "@/engine";
import { useShell } from "../context";
import { jobStore } from "../drawer/activity";
import { finished, itemEvent, report, started } from "../drawer/jobEvents";
import { renderShell } from "../test-utils";
import { OperationSheet } from "./OperationSheet";
import { useOperation } from "./useOperation";

/** A minimal operation: delete two dead letters of invoice-generate, or move them to label-print. */
function Harness({ move = false }: { move?: boolean }) {
  const { conn } = useShell();
  const [open, setOpen] = useState(true);
  const op = useOperation();
  if (!open) return <button onClick={() => setOpen(true)}>Reopen</button>;
  const form = { entity: "invoice-generate", view: "dead-letter" as const, selection: { type: "sequences" as const, sequences: ["41000", "41001"] } };
  return (
    <OperationSheet
      title={move ? "Move messages" : "Delete messages"}
      op={op}
      action={move ? "Move 2 messages" : "Delete 2 messages"}
      onSubmit={() => op.start(() => (move ? commands.opMove(conn.profileId!, form, "label-print", false) : commands.opDelete(conn.profileId!, form)))}
      onClose={() => setOpen(false)}
    >
      <p>The form</p>
    </OperationSheet>
  );
}

/** The shell has connected (the tree is behind the sheet, so it is not looked for). */
const connectedTo = (mock: { calls: { cmd: string }[] }) => waitFor(() => expect(mock.calls.some((c) => c.cmd === "list_entities")).toBe(true));

const ask = (p: Partial<ConfirmRequest> = {}): ConfirmRequest => ({
  id: "c1", job_id: "j1", verb: "delete", target: "invoice-generate", summary: "Delete 2 messages from invoice-generate/$DeadLetterQueue on Logistics",
  item_count: "2", reversible: false, production: true, step: 1, ...p,
});

/** The shell with the harness, whose operation starts job `j1` without running anything: the test drives its events. */
async function scripted(move = false) {
  const { mock, ...rest } = renderShell("default", { last_profile: "p-logistics" }, <Harness move={move} />);
  await connectedTo(mock);
  setTransport({
    listen: mock.listen,
    invoke: <T,>(cmd: string, args?: Record<string, unknown>) =>
      (cmd === "op_delete" || cmd === "op_move" ? Promise.resolve({ id: "j1" }) : mock.invoke(cmd, args)) as Promise<T>,
  });
  const sheet = await screen.findByRole("dialog", { name: move ? "Move messages" : "Delete messages" });
  return { mock, sheet, ...rest };
}

async function startJob(sheet: HTMLElement, label = "Delete 2 messages") {
  await userEvent.click(within(sheet).getByRole("button", { name: label }));
  act(() => jobStore.apply(started("j1", "Delete 2 from invoice-generate/$DeadLetterQueue", null, null, "delete")));
}

describe("OperationSheet", () => {
  it("confirm panel shows the second request after the first", async () => {
    const { sheet } = await scripted(true);
    await startJob(sheet, "Move 2 messages");
    const summary = "Move 2 messages from invoice-generate/$DeadLetterQueue to label-print on Logistics";
    act(() => jobStore.confirm(ask({ verb: "move", summary })));
    expect(within(sheet).getByText(summary)).toBeInTheDocument();
    expect(within(sheet).getByText("PROD")).toBeInTheDocument();
    expect(within(sheet).queryByText(/Confirmation 1/)).toBeNull();
    await userEvent.type(within(sheet).getByRole("textbox", { name: /Type invoice-generate to confirm/ }), "invoice-generate");
    await userEvent.click(within(sheet).getByRole("button", { name: "Move from invoice-generate" }));
    expect(within(sheet).queryByText(summary)).toBeNull();
    act(() => jobStore.confirm(ask({ id: "c2", verb: "move", summary, target: "label-print", step: 2 })));
    expect(within(sheet).getByText("Confirmation 2")).toBeInTheDocument();
    expect(within(sheet).getByText(summary)).toBeInTheDocument();
    const field = within(sheet).getByRole("textbox", { name: /Type label-print to confirm/ });
    expect(field).toHaveValue("");
    expect(within(sheet).getByRole("button", { name: "Move to label-print" })).toBeDisabled();
  });

  it("production confirm stays disabled until the typed name matches, and sends the typed text", async () => {
    const { mock, sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.confirm(ask()));
    const confirm = within(sheet).getByRole("button", { name: "Delete from invoice-generate" });
    expect(confirm).toBeDisabled();
    const field = within(sheet).getByRole("textbox", { name: /Type invoice-generate to confirm/ });
    await userEvent.type(field, "invoice-generat");
    expect(confirm).toBeDisabled();
    await userEvent.type(field, "E");
    expect(confirm).toBeDisabled();
    await userEvent.type(field, "{Backspace}e");
    expect(confirm).toBeEnabled();
    await userEvent.click(confirm);
    expect(mock.calls.find((c) => c.cmd === "confirm_answer")?.args).toEqual({ id: "c1", answer: { type: "typed", text: "invoice-generate" } });
  });

  it("off production one Confirm click allows it, with no typed field and no PROD", async () => {
    const { mock, sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.confirm(ask({ production: false })));
    expect(within(sheet).queryByRole("textbox")).toBeNull();
    expect(within(sheet).queryByText("PROD")).toBeNull();
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete from invoice-generate" }));
    expect(mock.calls.find((c) => c.cmd === "confirm_answer")?.args).toEqual({ id: "c1", answer: { type: "allow" } });
  });

  it("closing the sheet while confirming answers deny and the job ends cancelled", async () => {
    const { mock } = renderShell("operate-prod", { last_profile: "p-logistics" }, <Harness />);
    const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
    await connectedTo(mock);
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
    expect(await within(sheet).findByText(/^Delete 2 messages from invoice-generate/)).toBeInTheDocument();
    // No close button and no Escape while a confirmation waits; Cancel is the way out.
    expect(within(sheet).queryByRole("button", { name: "Close" })).toBeNull();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "Delete messages" })).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog", { name: "Delete messages" })).toBeNull();
    expect(mock.calls.some((c) => c.cmd === "job_cancel")).toBe(true);
    await waitFor(() => expect(jobStore.all()[0]).toMatchObject({ finished: true, error: { kind: "cancelled" }, report: null }));
  });

  it("the sheet is not dismissible while running, and Cancel job cancels it", async () => {
    const { mock, sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.apply(itemEvent("j1", { sequence_number: "41000", outcome: "succeeded" })));
    expect(within(sheet).queryByRole("button", { name: "Close" })).toBeNull();
    await userEvent.keyboard("{Escape}");
    expect(screen.getByRole("dialog", { name: "Delete messages" })).toBeInTheDocument();
    expect(within(sheet).getByText(/1 done/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Cancel job" }));
    expect(mock.calls.some((c) => c.cmd === "job_cancel" && (c.args as { id: string }).id === "j1")).toBe(true);
  });

  it("shows the report when the job is done, and Close closes", async () => {
    const { sheet } = await scripted();
    await startJob(sheet);
    act(() => jobStore.apply(finished("j1", { report: report({ items: 2, succeeded: 2, summary_line: "Deleted 2 messages from invoice-generate" }), summary: "Deleted 2 messages from invoice-generate" })));
    expect(await within(sheet).findByText("Deleted 2 messages from invoice-generate")).toBeInTheDocument();
    await userEvent.click(within(sheet).getAllByRole("button", { name: "Close" }).at(-1)!);
    expect(screen.queryByRole("dialog", { name: "Delete messages" })).toBeNull();
  });

  it("a refused start shows the problem in the form and stays on it", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" }, <Harness />);
    await connectedTo(mock);
    setTransport({
      listen: mock.listen,
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) =>
        (cmd === "op_delete" ? Promise.reject({ kind: "not_connected", message: "Choose a connection first." }) : mock.invoke(cmd, args)) as Promise<T>,
    });
    const sheet = await screen.findByRole("dialog", { name: "Delete messages" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Delete 2 messages" }));
    expect(await within(sheet).findByRole("alert")).toHaveTextContent("Choose a connection first.");
    expect(within(sheet).getByText("The form")).toBeInTheDocument();
  });
});

describe("ReportView", () => {
  const items: ItemView[] = [
    { sequence_number: "1", message_id: "a", session_id: null, entity: "orders", new_sequence_number: null, outcome: "succeeded", reason: null },
    { sequence_number: "2", message_id: "b", session_id: null, entity: "orders", new_sequence_number: null, outcome: "failed", reason: "lock lost" },
    { sequence_number: "9007199254740993", message_id: "c", session_id: null, entity: "orders", new_sequence_number: null, outcome: "missing", reason: "no longer in the view" },
  ];

  it("report view filters to failed items", async () => {
    const { mock, sheet } = await scripted();
    setTransport({
      listen: mock.listen,
      invoke: <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "op_delete") return Promise.resolve({ id: "j1" }) as Promise<T>;
        if (cmd === "job_report") {
          const a = args as { onlyProblems: boolean };
          const list = a.onlyProblems ? items.filter((i) => i.outcome !== "succeeded") : items;
          return Promise.resolve({ summary: report({ items: 3, succeeded: 1, failed: 1, missing: 1, incomplete: 2 }), offset: 0, total: list.length, items: list } satisfies ReportPage) as Promise<T>;
        }
        return mock.invoke(cmd, args);
      },
    });
    await startJob(sheet);
    act(() => jobStore.apply(finished("j1", { report: report({ outcome: "partly-failed", items: 3, succeeded: 1, failed: 1, missing: 1, incomplete: 2, summary_line: "Deleted 1 of 3 messages from orders; 1 failed" }) })));
    const grid = await within(sheet).findByRole("grid", { name: "Items" });
    await waitFor(() => expect(within(grid).getAllByRole("row")).toHaveLength(4));
    expect(within(grid).getByText("9007199254740993")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("radio", { name: /Failed/ }));
    await waitFor(() => expect(within(grid).getAllByRole("row")).toHaveLength(2));
    expect(within(grid).getByText("lock lost")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("radio", { name: /Not done/ }));
    await waitFor(() => expect(within(grid).getAllByRole("row")).toHaveLength(3));
  });
});
