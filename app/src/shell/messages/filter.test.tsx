import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
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
    expect(await within(bar).findByText(/unexpected `=` at column \d+/)).toBeInTheDocument();
    expect(within(bar).getByRole("button", { name: "Apply" })).toBeDisabled();
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
    await userEvent.click(within(bar).getByRole("button", { name: "Clear" }));
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
    expect(await within(bar).findByText(/unexpected `=` at column \d+/)).toBeInTheDocument();
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
    act(() => within(bar).getByRole("button", { name: "Clear" }).focus());
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
});
