import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";
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
    expect(sessionNote({ broker: { state: "listed", capped: false }, scanned: 48, truncated: false })).toBeNull();
    expect(sessionNote({ broker: { state: "listed", capped: true }, scanned: 48, truncated: false })).toBe("The service's list was cut at 10,000 sessions.");
    expect(sessionNote({ broker: { state: "empty" }, scanned: 1200, truncated: false })).toBe(
      "The service listed no sessions; showing sessions found in the first 1,200 messages.",
    );
    expect(sessionNote({ broker: { state: "unavailable", kind: "forbidden" }, scanned: 5, truncated: false })).toBe(
      "Couldn't ask the service for sessions; showing sessions found by peeking.",
    );
    expect(sessionNote({ broker: { state: "listed", capped: false }, scanned: 10_000, truncated: true })).toBe(
      "Message counts are lower bounds: only the first 10,000 messages were counted.",
    );
    expect(sessionNote({ broker: { state: "empty" }, scanned: 10_000, truncated: true })).toBe(
      "The service listed no sessions; showing sessions found in the first 10,000 messages. Message counts are lower bounds.",
    );
  });
});
