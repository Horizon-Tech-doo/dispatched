import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { setTransport, type IpcTransport } from "@/engine";
import { renderShell } from "../test-utils";
import { setViewState, viewKey } from "./viewStore";

const seed = {
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id: "invoice-generate", kind: "queue", subTab: "dead-letter" }], activeTab: "invoice-generate", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
};

const lastPeek = (mock: ReturnType<typeof renderShell>["mock"]) =>
  (mock.calls.filter((c) => c.cmd === "peek_page").at(-1)?.args as { request: { filter: Record<string, string> } }).request.filter;
const peeks = (mock: ReturnType<typeof renderShell>["mock"]) => mock.calls.filter((c) => c.cmd === "peek_page").length;
// In jsdom the resizable panels swallow pointer-downs, so focus moves explicitly (as in the tree tests).
const typeInto = async (el: HTMLElement, text: string) => {
  act(() => el.focus());
  await userEvent.keyboard(text);
};
const openBar = async () => {
  await userEvent.click(await screen.findByRole("button", { name: /Filter/ }));
  return screen.getByRole("search", { name: "Message filter" });
};

describe("filter bar", () => {
  it("shows the engine's problem under the field and blocks Apply", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Properties"), "sys.Subject == 'x'");
    // A plain sentence, then the engine's own words (without its prefix and backticks) on the detail line.
    expect(await within(bar).findByText("Use = to compare.", { selector: "p > span" })).toBeInTheDocument();
    expect(within(bar).getByText(/^unexpected = at column \d+$/)).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: "Apply" })).toBeDisabled();
    expect(within(bar).getByRole("status")).toHaveTextContent("Use = to compare.");
  });

  it("applies with Enter and counts the conditions on the toggle", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Body contains"), "SHP-1000{Enter}");
    await waitFor(() => expect(lastPeek(mock).body_contains).toBe("SHP-1000"));
    expect(screen.getByRole("button", { name: /Filter/ })).toHaveTextContent("1");
  });

  it("switches the body condition to a JSONPath with an expected value", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await userEvent.click(within(bar).getByRole("radio", { name: "JSONPath" }));
    await typeInto(within(bar).getByLabelText("JSONPath"), "$.status");
    await typeInto(within(bar).getByLabelText("Equals"), "held");
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(lastPeek(mock)).toMatchObject({ body_json_path: "$.status", body_json_equals: "held" }));
  });

  it("shows a session condition as a chip that can be removed", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid");
    setViewState(viewKey("p-logistics", "invoice-generate", "dead-letter"), { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: true, sorting: null, detailOpen: false }, { filter: { session_id: "RMA-1001" } });
    const chip = await screen.findByRole("button", { name: "Remove session RMA-1001" });
    await userEvent.click(chip);
    await waitFor(() => expect(lastPeek(mock).session_id ?? null).toBeNull());
  });

  it("Clear empties every condition", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Body contains"), "x{Enter}");
    await waitFor(() => expect(lastPeek(mock).body_contains).toBe("x"));
    expect(screen.getByRole("button", { name: /Filter/ })).toHaveTextContent("1");
    await userEvent.click(within(bar).getByRole("button", { name: "Clear filter" }));
    // The unfiltered view was read a moment ago and is still cached, so there may be no new peek: the applied
    // filter and the fields are what show it is cleared.
    await waitFor(() => expect(screen.getByRole("button", { name: /Filter/ })).not.toHaveTextContent("1"));
    expect(within(bar).getByLabelText("Body contains")).toHaveValue("");
  });

  it("does not send a bad filter when Enter comes before the check has run", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await screen.findByRole("grid");
    const before = peeks(mock);
    // Typed and submitted inside the 250 ms the check waits.
    await typeInto(within(bar).getByLabelText("Properties"), "sys.Subject == 'x'{Enter}");
    expect(await within(bar).findByText("Use = to compare.", { selector: "p > span" })).toBeInTheDocument();
    expect(peeks(mock)).toBe(before);
  });

  it("keeps text as typed and treats a blank (spaces only) field as no condition", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await userEvent.click(within(bar).getByRole("radio", { name: "Regex" }));
    await typeInto(within(bar).getByLabelText("Body regex"), "^a b $");
    await typeInto(within(bar).getByLabelText("Properties"), "   ");
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(lastPeek(mock).body_regex).toBe("^a b $"));
    expect(lastPeek(mock).property ?? null).toBeNull();
  });

  it("Enter on a button presses that button, not Apply", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Body contains"), "zzz");
    const before = peeks(mock);
    act(() => within(bar).getByRole("button", { name: "Clear filter" }).focus());
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(within(bar).getByLabelText("Body contains")).toHaveValue(""));
    await new Promise((r) => setTimeout(r, 50));
    expect(Object.values(lastPeek(mock)).filter(Boolean)).toHaveLength(0);
    expect(peeks(mock)).toBeLessThanOrEqual(before + 1);
  });

  it("Escape closes the bar and returns focus to the toggle; the filter stays applied", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    expect(within(bar).getByLabelText("Properties")).toHaveFocus();
    await typeInto(within(bar).getByLabelText("Body contains"), "SHP{Enter}");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("search", { name: "Message filter" })).toBeNull();
    const toggle = screen.getByRole("button", { name: /Filter/ });
    expect(toggle).toHaveFocus();
    expect(toggle).toHaveTextContent("1");
  });

  it("removing the session chip keeps what was typed", async () => {
    const { mock } = renderShell("default", seed);
    await screen.findByRole("grid");
    setViewState(viewKey("p-logistics", "invoice-generate", "dead-letter"), { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: true, sorting: null, detailOpen: false }, { filter: { session_id: "RMA-1001" } });
    const bar = await screen.findByRole("search", { name: "Message filter" });
    await typeInto(within(bar).getByLabelText("Body contains"), "draft");
    await userEvent.click(within(bar).getByRole("button", { name: "Remove session RMA-1001" }));
    await waitFor(() => expect(lastPeek(mock).session_id ?? null).toBeNull());
    expect(within(bar).getByLabelText("Body contains")).toHaveValue("draft");
  });

  it("runs a fresh peek when a recently used filter is applied again or cleared", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await screen.findByRole("grid");
    await typeInto(within(bar).getByLabelText("Body contains"), "SHP{Enter}");
    await waitFor(() => expect(lastPeek(mock).body_contains).toBe("SHP"));
    const withFilter = peeks(mock);
    await userEvent.click(within(bar).getByRole("button", { name: "Clear filter" }));
    await waitFor(() => expect(peeks(mock)).toBeGreaterThan(withFilter));
    expect(lastPeek(mock).body_contains ?? null).toBeNull();
    const cleared = peeks(mock);
    // The same filter again, a moment later: not the rows cached for it.
    await typeInto(within(bar).getByLabelText("Body contains"), "SHP{Enter}");
    await waitFor(() => expect(peeks(mock)).toBeGreaterThan(cleared));
    expect(lastPeek(mock).body_contains).toBe("SHP");
  });

  it("checks each settled text once", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Properties"), "sys.DeliveryCount > 3");
    await new Promise((r) => setTimeout(r, 450));
    expect(mock.calls.filter((c) => c.cmd === "check_filter")).toHaveLength(1);
  });

  it("holds Apply for a check that is still running", async () => {
    const { mock } = renderShell("default", seed);
    let release!: () => void;
    let checks = 0;
    const gate = new Promise<void>((r) => (release = r));
    const slow: IpcTransport = {
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "check_filter") {
          checks++;
          await gate;
        }
        return mock.invoke<T>(cmd, args);
      },
    };
    setTransport(slow);
    const bar = await openBar();
    await screen.findByRole("grid");
    const before = peeks(mock);
    await typeInto(within(bar).getByLabelText("Properties"), "sys.DeliveryCount > 3");
    await waitFor(() => expect(checks).toBe(1));
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    expect(peeks(mock)).toBe(before);
    await act(async () => release());
    await waitFor(() => expect(lastPeek(mock).property).toBe("sys.DeliveryCount > 3"));
  });

  it("keeps the body segment after applying with its field empty", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    await userEvent.click(within(bar).getByRole("radio", { name: "Regex" }));
    await typeInto(within(bar).getByLabelText("Body regex"), "abc{Enter}");
    await waitFor(() => expect(screen.getByRole("button", { name: /Filter/ })).toHaveTextContent("1"));
    await userEvent.click(within(bar).getByRole("radio", { name: "JSONPath" }));
    act(() => within(bar).getByLabelText("JSONPath").focus());
    await userEvent.clear(within(bar).getByLabelText("JSONPath"));
    await userEvent.click(within(bar).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /Filter/ })).not.toHaveTextContent("1"));
    expect(within(bar).getByRole("radio", { name: "JSONPath" })).toBeChecked();
  });

  it("applies only a changed filter, and says when the fields are not applied", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    const apply = within(bar).getByRole("button", { name: "Apply" });
    expect(apply).toBeDisabled();
    expect(within(bar).queryByText("Not applied")).toBeNull();
    await typeInto(within(bar).getByLabelText("Body contains"), "SHP");
    expect(within(bar).getByText("Not applied")).toBeInTheDocument();
    expect(apply).toBeEnabled();
    await userEvent.click(apply);
    await waitFor(() => expect(within(bar).queryByText("Not applied")).toBeNull());
    expect(apply).toBeDisabled();
  });

  it("shows the property syntax only while the field is focused, outside the bar's layout", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    const syntax = within(bar).getByText("sys.MessageId").closest("p")!;
    expect(within(bar).getByLabelText("Properties")).toHaveAccessibleDescription(/sys\.MessageId/);
    // It hangs under the bar and lets clicks through, so hiding it never moves a control being pressed.
    expect(syntax).toHaveClass("absolute", "pointer-events-none");
    act(() => within(bar).getByLabelText("Body contains").focus());
    expect(within(bar).queryByText("sys.MessageId")).toBeNull();
  });

  it("says how Equals compares while it is focused", async () => {
    renderShell("default", seed);
    const bar = await openBar();
    await userEvent.click(within(bar).getByRole("radio", { name: "JSONPath" }));
    expect(within(bar).queryByText(/Compared as JSON/)).toBeNull();
    act(() => within(bar).getByLabelText("Equals").focus());
    expect(within(bar).getByText(/Compared as JSON/)).toBeInTheDocument();
  });

  it("marks the toggle as controlling the bar, and focuses the first field after the chip is removed", async () => {
    renderShell("default", seed);
    await screen.findByRole("grid");
    const toggle = screen.getByRole("button", { name: /Filter/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    setViewState(viewKey("p-logistics", "invoice-generate", "dead-letter"), { from: { kind: "start" }, topN: 100, filter: {}, filterOpen: true, sorting: null, detailOpen: false }, { filter: { session_id: "RMA-1001" } });
    const bar = await screen.findByRole("search", { name: "Message filter" });
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(toggle).toHaveAttribute("aria-controls", bar.id);
    await userEvent.click(within(bar).getByRole("button", { name: "Remove session RMA-1001" }));
    await waitFor(() => expect(within(bar).getByLabelText("Properties")).toHaveFocus());
  });

  it("shows the applied filter after the Filter button when the bar is closed, and clears it from there", async () => {
    const { mock } = renderShell("default", seed);
    const bar = await openBar();
    await typeInto(within(bar).getByLabelText("Properties"), "attempt >= 2");
    await typeInto(within(bar).getByLabelText("Body contains"), "held{Enter}");
    await waitFor(() => expect(lastPeek(mock).body_contains).toBe("held"));
    await userEvent.keyboard("{Escape}");
    const summary = await screen.findByRole("button", { name: 'Edit filter: attempt >= 2 · body contains "held"' });
    expect(summary).toHaveTextContent('attempt >= 2 · body contains "held"');
    await userEvent.click(summary);
    expect(screen.getByRole("search", { name: "Message filter" })).toBeInTheDocument();
    // Open, the clear action is the bar's own: the one in the toolbar is gone.
    expect(screen.getAllByRole("button", { name: "Clear filter" })).toHaveLength(1);
    await userEvent.keyboard("{Escape}");
    await userEvent.click(await screen.findByRole("button", { name: "Clear filter" }));
    await waitFor(() => expect(screen.getByRole("button", { name: /Filter/ })).not.toHaveTextContent("2"));
    expect(screen.queryByRole("button", { name: /^Edit filter/ })).toBeNull();
  });

  it("treats a second Enter while Apply waits for the check as the same request", async () => {
    const { mock } = renderShell("default", seed);
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    let checks = 0;
    setTransport({
      listen: mock.listen,
      invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
        if (cmd === "check_filter") {
          checks++;
          await gate;
        }
        return mock.invoke<T>(cmd, args);
      },
    });
    const bar = await openBar();
    await screen.findByRole("grid");
    const before = peeks(mock);
    await typeInto(within(bar).getByLabelText("Properties"), "sys.DeliveryCount > 3{Enter}");
    await waitFor(() => expect(checks).toBe(1));
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard("{Enter}");
    await act(async () => release());
    await waitFor(() => expect(lastPeek(mock).property).toBe("sys.DeliveryCount > 3"));
    await new Promise((r) => setTimeout(r, 100));
    expect(peeks(mock)).toBe(before + 1);
  });
});
