import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { ReceiveForm } from "@/engine";
import { useShell } from "../context";
import { renderShell } from "../test-utils";
import { lastCall } from "./testing";

function Opener() {
  const { openSheet, live } = useShell();
  const [n, setN] = useState(0);
  return live ? (
    <button
      onClick={() => {
        setN(n + 1);
        openSheet({ kind: "operation", op: "receive", entity: "label-print", subQueue: "active", session: null });
      }}
    >
      Open receive
    </button>
  ) : null;
}

describe("ReceiveSheet", () => {
  it("receive-and-delete is off when the sheet opens, even after a previous delete run", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, <Opener />);
    await userEvent.click(await screen.findByRole("button", { name: "Open receive" }));
    let sheet = await screen.findByRole("dialog", { name: "Receive messages" });
    expect(within(sheet).getByRole("radio", { name: "Peek-lock" })).toBeChecked();
    await userEvent.click(within(sheet).getByRole("radio", { name: "Receive and delete" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Receive and delete up to 10 messages" }));
    await waitFor(() => expect(lastCall(mock, "op_receive")).toBeDefined());
    expect(lastCall<{ form: ReceiveForm }>(mock, "op_receive")!.form).toMatchObject({ entity: "label-print", sub_queue: "active", max: 10, action: { type: "delete" } });
    // Answer, let it finish, close; then open the sheet again.
    await userEvent.click(await within(sheet).findByRole("button", { name: "Receive and delete from label-print" }));
    await userEvent.click((await within(sheet).findAllByRole("button", { name: "Close" })).at(-1)!);
    await userEvent.click(await screen.findByRole("button", { name: "Open receive" }));
    sheet = await screen.findByRole("dialog", { name: "Receive messages" });
    expect(within(sheet).getByRole("radio", { name: "Peek-lock" })).toBeChecked();
    expect(within(sheet).getByRole("radio", { name: "Receive and delete" })).not.toBeChecked();
  });

  it("peek-lock sends the chosen settlement, with a dead-letter reason", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, <Opener />);
    await userEvent.click(await screen.findByRole("button", { name: "Open receive" }));
    const sheet = await screen.findByRole("dialog", { name: "Receive messages" });
    await userEvent.click(within(sheet).getByRole("radio", { name: "Dead-letter" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Receive up to 10 messages" }));
    await waitFor(() => expect(lastCall(mock, "op_receive")).toBeDefined());
    expect(lastCall<{ form: ReceiveForm }>(mock, "op_receive")!.form.action).toEqual({ type: "settle", how: { type: "dead-letter" } });
  });
});
