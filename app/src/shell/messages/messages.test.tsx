import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, setTransport, type IpcTransport, type MessageSummary, type PeekPageView, type PeekRequest } from "@/engine";
import { renderShell } from "../test-utils";
import { setViewState, viewKey, type ViewState } from "./viewStore";

const TRACKING = "shipment-events/subscriptions/tracking-sync";

/** The mock behind `renderShell`, with `edit` changing a command's answer (or holding it) before it returns. */
function around(mock: IpcTransport, edit: (cmd: string, args: Record<string, unknown> | undefined, answer: () => Promise<unknown>) => Promise<unknown>) {
  setTransport({ listen: mock.listen, invoke: <T,>(cmd: string, args?: Record<string, unknown>) => edit(cmd, args, () => mock.invoke(cmd, args)) as Promise<T> });
}

const VIEW_DEFAULTS: ViewState = { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: false, sorting: null, detailOpen: false };
/** Start a view with some of its state set, as if the user had chosen it. */
const presetView = (tabId: string, view: string, patch: Partial<ViewState>) => setViewState(viewKey("p-logistics", tabId, view), VIEW_DEFAULTS, patch);

const summary = (seq: number): MessageSummary => ({
  sequence_number: seq, state: "active", message_id: `m-${seq}`, correlation_id: null, session_id: null, subject: "s", content_type: null, to: null,
  reply_to: null, reply_to_session_id: null, partition_key: null, expires_at: null, enqueued_time: "2026-10-15T09:00:00Z", scheduled_enqueue_time: null, delivery_count: 0, time_to_live_ms: null,
  dead_letter_reason: null, dead_letter_description: null, dead_letter_source: null, application_properties: {}, body_kind: "empty", body_size: 0,
  body_preview: null,
});

const seedTab = (id: string, kind: "queue" | "subscription", subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

const grid = () => screen.findByRole("grid", { name: /dead-letter|messages/i });

describe("message views", () => {
  it("lists dead letters with reasons, counts and the read-only note", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    expect(await within(g).findAllByText("MaxDeliveryCountExceeded")).not.toHaveLength(0);
    const status = screen.getByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("37 messages · end of the dead-letter queue"));
    // The global status bar says it once; the view does not repeat it.
    expect(status).not.toHaveTextContent("Peek only");
  });

  it("peek more reads the next Top N", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
  });

  it("from the end shows the newest messages and no Peek more", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    await grid();
    await userEvent.click(screen.getByRole("radio", { name: "End" }));
    const status = screen.getByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("Last 100 messages"));
    expect(within(status).queryByRole("button", { name: "Peek more" })).toBeNull();
    // The last 100 dead letters are 48114..48213, oldest first.
    expect(within(await grid()).getAllByRole("row")[1]).toHaveTextContent("48114");
  });

  it("sorts by sequence number descending", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    const seqHeader = within(g).getByRole("columnheader", { name: /Seq/ });
    await userEvent.click(within(seqHeader).getByRole("button"));
    await userEvent.click(within(seqHeader).getByRole("button"));
    const first = within(g).getAllByRole("row")[1];
    expect(first).toHaveTextContent("41036");
  });

  it("hides a column from the Columns menu and remembers it", async () => {
    const { mock } = renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    // In jsdom the resizable panels swallow the pointer-down Radix opens menus on: open it by keyboard.
    act(() => screen.getByRole("button", { name: "Columns" }).focus());
    await userEvent.keyboard("{Enter}");
    await userEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Description" }));
    await waitFor(() => expect(within(g).queryByRole("columnheader", { name: "Description" })).toBeNull());
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "columns")).toBe(true));
  });

  it("shows a peek error with the fix", async () => {
    renderShell("peek-error", seedTab("invoice-generate", "queue", "dead-letter"));
    expect(await screen.findByRole("heading", { name: "Can't read the dead-letter queue" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Try again" })).toBeInTheDocument();
  });

  it("an empty view says so in plain words", async () => {
    renderShell("default", seedTab("label-print", "queue", "dead-letter"));
    expect(await screen.findByRole("heading", { name: "No dead letters" })).toBeInTheDocument();
    // The empty state says it; the status line has nothing to add.
    expect(screen.getByRole("status", { name: "Peek status" })).toHaveTextContent(/^$/);
  });

  it("peek again after Peek more reads Top N again, not the grown total", async () => {
    renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
    await userEvent.click(screen.getByRole("button", { name: "Peek again" }));
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    await new Promise((r) => setTimeout(r, 50));
    expect(status).toHaveTextContent("100 messages");
  });

  it("starts each test with fresh view state", async () => {
    // Sorting set by an earlier test must not carry over (the view store is module state).
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    expect(within(g).getByRole("columnheader", { name: /Seq/ })).not.toHaveAttribute("aria-sort");
  });

  it("keeps the rows when a later page fails, and Try again continues the round", async () => {
    const { mock } = renderShell("default", seedTab("shipment-events/subscriptions/tracking-sync", "subscription", "dead-letter"));
    let peeks = 0;
    let failNext = false;
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "peek_page") {
          peeks++;
          if (failNext) {
            failNext = false;
            throw new EngineError("network", "network error: connection reset by peer");
          }
        }
        return mock.invoke<T>(cmd, args);
      },
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages"));
    failNext = true;
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    const alert = await screen.findByRole("alert");
    expect(within(alert).getByRole("heading", { name: "Couldn't reach the namespace" })).toBeInTheDocument();
    expect(status).toHaveTextContent("100 messages · the next page failed");
    expect(within(await grid()).getAllByRole("row").length).toBeGreaterThan(1);
    const before = peeks;
    await userEvent.click(within(alert).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(status).toHaveTextContent("200 messages"));
    expect(peeks).toBe(before + 1);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows the round's progress while it streams, and Stop in the toolbar ends it", async () => {
    const { mock } = renderShell("default", seedTab(TRACKING, "subscription", "dead-letter"));
    presetView(TRACKING, "dead-letter", { topN: 5000 });
    let pages = 0;
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    around(mock, async (cmd, _args, answer) => {
      if (cmd === "peek_page" && ++pages === 2) await gate;
      return answer();
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("500 of 5,000 · reading…"));
    const toolbar = screen.getByRole("toolbar", { name: "Peek options" });
    expect(within(toolbar).queryByRole("button", { name: "Peek again" })).toBeNull();
    const stop = within(toolbar).getByRole("button", { name: "Stop" });
    // A filled square (a stop symbol), not an outlined one that reads like a checkbox; and the slot is wide enough
    // for the longer label, so Columns does not move.
    expect(stop.querySelector("svg")).toHaveClass("fill-current");
    expect(stop.closest("[data-peek-slot]")).toHaveClass("min-w-[104px]");
    await userEvent.click(stop);
    release();
    await waitFor(() => expect(status).toHaveTextContent("1,000 messages · stopped"));
    expect(within(toolbar).getByRole("button", { name: "Peek again" })).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 30));
    expect(pages).toBe(2);
  });

  it("says when a scan from the end stopped before the newest messages", async () => {
    const { mock } = renderShell("default", seedTab(TRACKING, "subscription", "dead-letter"));
    presetView(TRACKING, "dead-letter", { from: { kind: "end" } });
    around(mock, async (cmd, args, answer) => {
      const page = (await answer()) as PeekPageView;
      return cmd === "peek_page" && (args?.request as PeekRequest).from.kind === "end" ? { ...page, exhausted: false } : page;
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("100 messages · the scan stopped before the end, so these are not the newest"));
  });

  it("says older messages may not have been examined when a scan from the end hit its limit", async () => {
    const { mock } = renderShell("default", seedTab(TRACKING, "subscription", "dead-letter"));
    presetView(TRACKING, "dead-letter", { from: { kind: "end" } });
    around(mock, async (cmd, args, answer) => {
      const page = (await answer()) as PeekPageView;
      if (cmd !== "peek_page" || (args?.request as PeekRequest).from.kind !== "end") return page;
      return { ...page, messages: page.messages.slice(-3), scanned: 100_000, scan_limit: 100_000, exhausted: true };
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("3 matching messages in the newest 100,000 · older messages may not have been examined"));
  });

  it("holds 100,000 rows: Top N 50,000 can peek more once", async () => {
    const { mock } = renderShell("default", seedTab(TRACKING, "subscription", "dead-letter"));
    presetView(TRACKING, "dead-letter", { topN: 50_000 });
    around(mock, async (cmd, args, answer) => {
      if (cmd !== "peek_page") return answer();
      const r = args!.request as PeekRequest;
      const from = r.from.kind === "sequence" ? r.from.sequence : 1;
      const messages = Array.from({ length: r.max }, (_, i) => summary(from + i));
      return { messages, scanned: r.max, scan_limit: r.scan_limit ?? 5000, next_sequence: from + r.max, exhausted: false } satisfies PeekPageView;
    });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("50,000 messages · more available"), { timeout: 10_000 });
    await userEvent.click(within(status).getByRole("button", { name: "Peek more" }));
    await waitFor(() => expect(status).toHaveTextContent("100,000 messages · the view holds up to 100,000 messages"), { timeout: 10_000 });
    expect(within(status).queryByRole("button", { name: "Peek more" })).toBeNull();
  }, 30_000);

  it("offers only the columns the view can fill, and resets them", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "messages"));
    const g = await grid();
    act(() => screen.getByRole("button", { name: "Columns" }).focus());
    await userEvent.keyboard("{Enter}");
    await screen.findByRole("menuitemcheckbox", { name: "Subject" });
    expect(screen.queryByRole("menuitemcheckbox", { name: "Reason" })).toBeNull();
    expect(screen.queryByRole("menuitemcheckbox", { name: "Session" })).toBeNull();
    await userEvent.click(screen.getByRole("menuitemcheckbox", { name: "Subject" }));
    await waitFor(() => expect(within(g).queryByRole("columnheader", { name: "Subject" })).toBeNull());
    await userEvent.click(screen.getByRole("menuitem", { name: "Reset columns" }));
    await waitFor(() => expect(within(g).getByRole("columnheader", { name: "Subject" })).toBeInTheDocument());
  });

  it("keeps a saved column this entity cannot show, so another entity of the group still gets it", async () => {
    // Columns are saved per view group: Session was added from a session entity's Messages view.
    const { mock } = renderShell("default", { ...seedTab("invoice-generate", "queue", "messages"), columns: { active: ["seq", "session", "subject", "prop:region"] } });
    const g = await grid();
    await waitFor(() => expect(within(g).getByRole("columnheader", { name: "Subject" })).toBeInTheDocument());
    expect(within(g).queryByRole("columnheader", { name: "Session" })).toBeNull();
    act(() => screen.getByRole("button", { name: "Columns" }).focus());
    await userEvent.keyboard("{Enter}");
    await userEvent.click(await screen.findByRole("menuitemcheckbox", { name: "Subject" }));
    await waitFor(() => expect(within(g).queryByRole("columnheader", { name: "Subject" })).toBeNull());
    const saved = mock.calls.filter((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "columns").at(-1)!.args as { value: { active: string[] } };
    expect(saved.value.active).toEqual(["seq", "session", "prop:region"]);
  });

  it("clears the selection when the peek starts again", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    await userEvent.click(within(g).getAllByRole("row")[1]);
    expect(within(g).getAllByRole("row")[1]).toHaveAttribute("aria-selected", "true");
    await userEvent.click(screen.getByRole("button", { name: "Peek again" }));
    await waitFor(() => expect(g.querySelectorAll('[aria-selected="true"]')).toHaveLength(0));
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    expect(g.querySelectorAll('[aria-selected="true"]')).toHaveLength(0);
  });

  it("refuses a sequence number too large to type, and says so", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    await grid();
    await userEvent.click(screen.getByRole("radio", { name: "Sequence" }));
    const field = screen.getByRole("textbox", { name: "From sequence number" });
    await userEvent.type(field, "123456789012345678901{Enter}");
    expect(field).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByRole("alert")).toHaveTextContent(/too large/i);
    expect(screen.getByRole("radio", { name: "Sequence" })).toHaveAttribute("aria-checked", "true");
    await userEvent.clear(field);
    await userEvent.type(field, "41030{Enter}");
    expect(field).not.toHaveAttribute("aria-invalid");
    await waitFor(() => expect(screen.getByRole("status", { name: "Peek status" })).toHaveTextContent("7 messages"));
  });

  it("explains an empty scheduled view the listing counts as not empty", async () => {
    const { mock } = renderShell("default", seedTab("invoice-generate", "queue", "scheduled"));
    around(mock, async (cmd, args, answer) => {
      const v = await answer();
      if (cmd === "list_entities") {
        const l = v as { queues: { name: string; counts: { scheduled: number } | null }[] };
        for (const q of l.queues) if (q.name === "invoice-generate" && q.counts) q.counts = { ...q.counts, scheduled: 3 };
      }
      if (cmd === "peek_page" && (args?.request as PeekRequest).view === "scheduled") return { ...(v as PeekPageView), messages: [] };
      return v;
    });
    expect(await screen.findByRole("heading", { name: "No scheduled messages" })).toBeInTheDocument();
    expect(screen.getByText(/they may have become active since/)).toBeInTheDocument();
  });

  it("shows compact enqueued times with the full time on hover", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    const cell = within(within(g).getAllByRole("row")[1]).getAllByRole("gridcell")[2];
    expect(cell.textContent).toMatch(/^\d{2} [A-Z][a-z]{2} \d{2}:\d{2}:\d{2}$/);
    expect(cell.querySelector("[title]")?.getAttribute("title")).toMatch(/^2026-10-15T/);
  });

  it("reads the Peek size setting as each view's first Top N", async () => {
    renderShell("default", { ...seedTab(TRACKING, "subscription", "dead-letter"), ui: { peekSize: 500 } });
    const status = await screen.findByRole("status", { name: "Peek status" });
    await waitFor(() => expect(status).toHaveTextContent("500 messages · more available"));
    await userEvent.keyboard("{Control>},{/Control}");
    const sheet = await screen.findByRole("dialog", { name: "Settings" });
    const size = within(sheet).getByRole("radiogroup", { name: "Peek size" });
    expect(within(size).getByRole("radio", { name: "500" })).toHaveAttribute("aria-checked", "true");
    await userEvent.click(within(size).getByRole("radio", { name: "1,000" }));
    await waitFor(() => expect(status).toHaveTextContent("1,000 messages"));
  });

  it("forgets a connection's view state when switching to another connection", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    let g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    await userEvent.click(within(within(g).getByRole("columnheader", { name: /Seq/ })).getByRole("button"));
    expect(within(g).getByRole("columnheader", { name: /Seq/ })).toHaveAttribute("aria-sort", "ascending");
    const steps: [RegExp, RegExp, RegExp][] = [
      [/^Connection: Logistics/, /Partner gateway/, /^Connection: Partner/],
      [/^Connection: Partner/, /^Logistics sb-logistics-prod/, /^Connection: Logistics/],
    ];
    for (const [selector, item, after] of steps) {
      act(() => screen.getByRole("button", { name: selector }).focus());
      await userEvent.keyboard("{Enter}");
      await userEvent.click(await screen.findByRole("menuitem", { name: item }));
      await screen.findByRole("button", { name: after });
    }
    g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    expect(within(g).getByRole("columnheader", { name: /Seq/ })).not.toHaveAttribute("aria-sort");
  });

  it("opens the detail with Enter and closes it with Escape", async () => {
    renderShell("default", seedTab("invoice-generate", "queue", "dead-letter"));
    const g = await grid();
    await within(g).findAllByText("MaxDeliveryCountExceeded");
    const row = within(g).getAllByRole("row")[1];
    const seq = row.dataset.rowId!;
    await userEvent.click(row);
    // In jsdom the resizable panels swallow the pointer-down that would focus the grid.
    act(() => g.focus());
    expect(screen.queryByRole("region", { name: /^Message/ })).toBeNull();
    await userEvent.keyboard("{Enter}");
    const pane = await screen.findByRole("region", { name: `Message ${seq}` });
    await within(pane).findByRole("tab", { name: "Body" });
    // The pane follows the focused row.
    await userEvent.keyboard("{ArrowDown}");
    const next = await screen.findByRole("region", { name: `Message ${Number(seq) + 1}` });
    await within(next).findByRole("tab", { name: "Body" });
    act(() => void within(screen.getByRole("region", { name: /^Message/ })).getByRole("tab", { name: "Body" }).focus());
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("region", { name: /^Message/ })).toBeNull();
    // The keyboard goes back to the grid.
    expect(g).toHaveFocus();
  });
});
