import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { lastCall } from "../operate/testing";
import { OpenSheet } from "../operate/testing";
import { renderShell } from "../test-utils";

const open = (session = "RMA-1001") => <OpenSheet sheet={{ kind: "operation", op: "session-state", entity: "returns-intake", session }} />;

describe("SessionStateSheet", () => {
  it("shows the current state as text", async () => {
    renderShell("operate-ok", { last_profile: "p-logistics" }, open());
    const sheet = await screen.findByRole("dialog", { name: "Session state" });
    expect(await within(sheet).findByText(/"step":"inspect"/)).toBeInTheDocument();
    expect(within(sheet).getByText(/41 B/)).toBeInTheDocument();
  });

  it("session state clear offers copy first", async () => {
    const write = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, open());
    const sheet = await screen.findByRole("dialog", { name: "Session state" });
    await within(sheet).findByText(/"step":"inspect"/);
    await userEvent.click(within(sheet).getByRole("radio", { name: "Clear" }));
    expect(within(sheet).getByText(/Copy the current state first/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Copy current state" }));
    expect(write).toHaveBeenCalledWith('{"step":"inspect","owner":"returns-team"}');
    await userEvent.click(within(sheet).getByRole("button", { name: "Clear state" }));
    await waitFor(() => expect(lastCall(mock, "op_session_state_set")).toEqual({ profileId: "p-logistics", entity: "returns-intake", session: "RMA-1001", value: null, allowLarge: false }));
  });

  it("sets text as base64, and state over 256 KiB needs Allow large state", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, open("RMA-1002"));
    const sheet = await screen.findByRole("dialog", { name: "Session state" });
    expect(await within(sheet).findByText("This session has no state.")).toBeInTheDocument();
    const field = within(sheet).getByRole("textbox", { name: "New state" });
    await act(async () => field.focus());
    await userEvent.paste("x".repeat(256 * 1024 + 1));
    expect(within(sheet).getByRole("button", { name: "Replace state" })).toBeDisabled();
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /Allow large state/ }));
    await userEvent.clear(field);
    await userEvent.paste("hé");
    await userEvent.click(within(sheet).getByRole("button", { name: "Replace state" }));
    await waitFor(() => expect(lastCall(mock, "op_session_state_set")).toMatchObject({ value: "aMOp", allowLarge: true }));
  });
});
