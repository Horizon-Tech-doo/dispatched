import { QueryClient } from "@tanstack/react-query";
import { act, renderHook, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { peekSession } from "../messages/peekSession";
import { setViewState, useViewState, viewKey, defaultViewState } from "../messages/viewStore";
import { renderShell } from "../test-utils";
import type { SessionListing } from "@/engine";
import { sessionNote } from "./SessionsView";

const seed = (id: string, kind: string, subTab: string) => ({
  last_profile: "p-logistics",
  "shell:p-logistics": { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

describe("sessions", () => {
  it("lists sessions with counts, including an empty one, and peeks one", async () => {
    const { mock } = renderShell("default", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    // The grid shows at once, reading; its rows follow.
    const empty = await within(grid).findByRole("row", { name: /RMA-0999/ });
    expect(within(empty).getByText("0")).toBeInTheDocument();
    await userEvent.click(within(grid).getByRole("row", { name: /RMA-1003/ }));
    await userEvent.click(screen.getByRole("button", { name: "Peek messages" }));
    expect(await screen.findByRole("tab", { name: /Messages/, selected: true })).toBeInTheDocument();
    await waitFor(() => {
      const req = mock.calls.filter((c) => c.cmd === "peek_page").at(-1)?.args as { request: { filter: { session_id?: string } } } | undefined;
      expect(req?.request.filter.session_id).toBe("RMA-1003");
    });
    expect(screen.getByRole("button", { name: "Remove session RMA-1003" })).toBeInTheDocument();
  });

  it("peeks a session with Enter on its row", async () => {
    renderShell("default", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    await userEvent.click(await within(grid).findByRole("row", { name: /RMA-1002/ }));
    act(() => grid.focus());
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("button", { name: "Remove session RMA-1002" })).toBeInTheDocument();
  });

  it("filters sessions by name", async () => {
    renderShell("default", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    await within(grid).findByRole("row", { name: /RMA-0999/ });
    // In jsdom the resizable panels swallow the pointer-down that would focus the field: focus it directly.
    act(() => screen.getByRole("textbox", { name: "Filter sessions" }).focus());
    await userEvent.keyboard("0999");
    await waitFor(() => expect(within(grid).getAllByRole("row")).toHaveLength(2));
    await userEvent.keyboard("zzz");
    expect(await screen.findByRole("heading", { name: "No session matches" })).toBeInTheDocument();
  });

  it("logs the listing in Activity", async () => {
    renderShell("default", {
      last_profile: "p-logistics",
      "shell:p-logistics": { tabs: [{ id: "returns-intake", kind: "queue", subTab: "sessions" }], activeTab: "returns-intake", expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: true, drawerSize: 18, drawerTab: "activity" } },
    });
    expect(await screen.findByText("Listed 13 sessions of returns-intake")).toBeInTheDocument();
  });

  it("says when an entity does not use sessions", async () => {
    renderShell("default", seed("invoice-generate", "queue", "sessions"));
    expect(await screen.findByRole("heading", { name: "This queue doesn't use sessions" })).toBeInTheDocument();
  });

  it("words the source of the list", () => {
    const l = (broker: SessionListing["broker"], scanned: number, truncated: boolean) => ({ broker, scanned, truncated });
    expect(sessionNote(l({ state: "listed", capped: false }, 48, false))).toEqual([]);
    expect(sessionNote(l({ state: "listed", capped: true }, 48, false))).toEqual(["The service's list was cut at 10,000 sessions."]);
    expect(sessionNote(l({ state: "listed", capped: false }, 10_000, true))).toEqual(["counts from the first 10,000 messages are lower bounds"]);
    expect(sessionNote(l({ state: "empty" }, 1200, false))).toEqual(["The service listed no sessions; showing sessions found in the first 1,200 messages."]);
    expect(sessionNote(l({ state: "unavailable", kind: "forbidden" }, 5, false))).toEqual(["Couldn't ask the service for sessions; showing sessions found by peeking."]);
    // A cut tally behind an empty or failed list may also have missed sessions.
    expect(sessionNote(l({ state: "empty" }, 10_000, true))).toEqual([
      "The service listed no sessions; showing sessions found in the first 10,000 messages. Some sessions may be missing.",
      "counts are lower bounds",
    ]);
    expect(sessionNote(l({ state: "unavailable", kind: "network" }, 10_000, true))).toEqual([
      "Couldn't ask the service for sessions; showing sessions found by peeking. Some sessions may be missing.",
      "counts are lower bounds",
    ]);
  });

  it("says in its status line how many sessions the service listed", async () => {
    renderShell("default", seed("returns-intake", "queue", "sessions"));
    const status = await screen.findByRole("status", { name: "Sessions status" });
    await waitFor(() => expect(status).toHaveTextContent("13 sessions · 13 listed by the service"));
    expect(screen.queryByText(/Session state needs/)).toBeNull();
    act(() => screen.getByRole("textbox", { name: "Filter sessions" }).focus());
    await userEvent.keyboard("0999");
    await waitFor(() => expect(status).toHaveTextContent("1 of 13 sessions"));
  });

  it("marks the sessions the service listed", async () => {
    renderShell("default", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    expect(within(grid).getByRole("columnheader", { name: "Listed by service" })).toBeInTheDocument();
    const row = await within(grid).findByRole("row", { name: /RMA-0999/ });
    expect(within(row).getByText("Yes")).toBeInTheDocument();
  });

  it("explains a list the service did not give", async () => {
    renderShell("sessions-empty", seed("returns-intake", "queue", "sessions"));
    const status = await screen.findByRole("status", { name: "Sessions status" });
    await waitFor(() => expect(status).toHaveTextContent("The service listed no sessions; showing sessions found in the first 48 messages."));
    expect(status).not.toHaveTextContent("listed by the service ·");
    const grid = screen.getByRole("grid", { name: "Sessions" });
    const row = await within(grid).findByRole("row", { name: /RMA-1001/ });
    expect(within(row).getByText("Not listed by the service")).toBeInTheDocument();
    expect(within(grid).queryByRole("row", { name: /RMA-0999/ })).toBeNull();
  });

  it("notes a capped service list", async () => {
    renderShell("sessions-capped", seed("returns-intake", "queue", "sessions"));
    const status = await screen.findByRole("status", { name: "Sessions status" });
    await waitFor(() => expect(status).toHaveTextContent("The service's list was cut at 10,000 sessions."));
  });

  it("shows counts from a cut tally as lower bounds", async () => {
    renderShell("sessions-cut", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    const row = await within(grid).findByRole("row", { name: /RMA-1001/ });
    expect(within(row).getByText("~4+")).toBeInTheDocument();
    const status = screen.getByRole("status", { name: "Sessions status" });
    expect(status).toHaveTextContent("Couldn't ask the service for sessions; showing sessions found by peeking. Some sessions may be missing.");
    expect(status).toHaveTextContent("counts are lower bounds");
  });

  it("disables Peek messages when the focused session is filtered out", async () => {
    renderShell("default", seed("returns-intake", "queue", "sessions"));
    const grid = await screen.findByRole("grid", { name: "Sessions" });
    await userEvent.click(await within(grid).findByRole("row", { name: /RMA-1003/ }));
    expect(screen.getByRole("button", { name: "Peek messages" })).toBeEnabled();
    act(() => screen.getByRole("textbox", { name: "Filter sessions" }).focus());
    await userEvent.keyboard("0999");
    await waitFor(() => expect(screen.getByRole("button", { name: "Peek messages" })).toBeDisabled());
  });

  it("peeks A, B, A and reads A afresh the second time", async () => {
    const { mock } = renderShell("default", seed("returns-intake", "queue", "sessions"));
    const peek = async (id: RegExp) => {
      // In jsdom the pointer-down on a tab is swallowed: focusing a tab selects it.
      const tab = await screen.findByRole("tab", { name: "Sessions" });
      act(() => {
        tab.blur(); // it may still hold focus from the last round, and focus only selects when it arrives
        tab.focus();
      });
      const grid = await screen.findByRole("grid", { name: "Sessions" });
      await userEvent.click(await within(grid).findByRole("row", { name: id }));
      await userEvent.click(screen.getByRole("button", { name: "Peek messages" }));
      await screen.findByRole("tab", { name: /Messages/, selected: true });
    };
    const peeksOf = (sid: string) => mock.calls.filter((c) => c.cmd === "peek_page" && (c.args as { request: { filter: { session_id?: string } } }).request.filter.session_id === sid).length;
    await peek(/RMA-1001/);
    await waitFor(() => expect(peeksOf("RMA-1001")).toBe(1));
    await peek(/RMA-1002/);
    await waitFor(() => expect(peeksOf("RMA-1002")).toBe(1));
    await peek(/RMA-1001/);
    await waitFor(() => expect(peeksOf("RMA-1001")).toBe(2));
  });
});

describe("peekSession", () => {
  it("replaces the other conditions and drops unobserved peeks of the view", () => {
    const qc = new QueryClient();
    const key = viewKey("p1", "orders", "active");
    const defaults = defaultViewState(100);
    setViewState(key, defaults, { filter: { property: "sys.DeliveryCount > 3", body_contains: "x" }, topN: 500 });
    qc.setQueryData(["peek", "p1", "orders", "active", { kind: "start" }, { session_id: "A" }, 100], { pages: [] });
    qc.setQueryData(["peek", "p1", "orders", "dead-letter", { kind: "start" }, {}, 100], { pages: [] });
    peekSession(qc, "p1", "orders", "A", 100);
    const { result } = renderHook(() => useViewState(key, defaults));
    expect(result.current[0]).toMatchObject({ filter: { session_id: "A" }, filterOpen: true, topN: 500 });
    expect(qc.getQueryCache().findAll({ queryKey: ["peek", "p1", "orders", "active"] })).toHaveLength(0);
    expect(qc.getQueryCache().findAll({ queryKey: ["peek", "p1", "orders", "dead-letter"] })).toHaveLength(1);
  });
});
