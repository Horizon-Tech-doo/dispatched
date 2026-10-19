import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { MessageDetail, ReceiveForm, SendForm } from "@/engine";
import { applyTheme } from "@/ui";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "./testing";
import type { OperationSeed } from "./seed";

const open = (seed: OperationSeed, scenario: Parameters<typeof renderShell>[0] = "operate-ok") =>
  renderShell(scenario, { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);

describe("operation sheets after review", () => {
  it("a pristine composer without an entity shows no error, only a disabled Send", async () => {
    open({ op: "send", entity: null });
    const sheet = await screen.findByRole("dialog", { name: "Send messages" });
    expect(within(sheet).queryByText(/Choose where to send/)).toBeNull();
    expect(sheet.querySelector(".text-accent")).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Send 1 message" })).toBeDisabled();
    // The target is picked from the namespace, not typed blind.
    await userEvent.click(await within(sheet).findByRole("option", { name: "label-print" }));
    expect(within(sheet).getByRole("button", { name: "Send 1 message" })).toBeEnabled();
  });

  it("receive opens with no settlement chosen; the primary waits for one and names it", async () => {
    const { mock } = open({ op: "receive", entity: "label-print", subQueue: "active", session: null });
    const sheet = await screen.findByRole("dialog", { name: "Receive messages" });
    for (const name of ["Complete", "Abandon", "Defer", "Dead-letter"]) expect(within(sheet).getByRole("radio", { name })).not.toBeChecked();
    expect(within(sheet).getByRole("radio", { name: "Receive and delete" })).not.toBeChecked();
    expect(within(sheet).getByText("Choose what happens to each message.")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Receive up to 10 messages" })).toBeDisabled();
    await userEvent.click(within(sheet).getByRole("radio", { name: "Abandon" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Receive and abandon up to 10 messages" }));
    await waitFor(() => expect(lastCall<{ form: ReceiveForm }>(mock, "op_receive")?.form.action).toEqual({ type: "settle", how: { type: "abandon" } }));
  });

  it("resubmit says what happens to the originals and picks the target with a segmented control", async () => {
    open({ op: "resubmit", entity: "invoice-generate", view: "dead-letter", sequences: [41000], filter: {} });
    const sheet = await screen.findByRole("dialog", { name: "Resubmit messages" });
    expect(within(sheet).getByText("Copies are sent as new messages and the originals are removed.")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /Remove the originals/ }));
    expect(within(sheet).getByText("Copies are sent as new messages; the originals stay.")).toBeInTheDocument();
    expect(within(sheet).getByRole("radio", { name: "Back to invoice-generate" })).toBeChecked();
    // The edits wait behind a disclosure.
    expect(within(sheet).getByRole("button", { name: /^Edits/ })).toHaveAttribute("aria-expanded", "false");
    expect(within(sheet).queryByRole("textbox", { name: "Subject" })).toBeNull();
    await userEvent.click(within(sheet).getByRole("radio", { name: "Another entity" }));
    expect(within(sheet).getByRole("listbox", { name: "Resubmit to" })).toBeInTheDocument();
  });

  it("cancelling scheduled messages is called Unschedule, never Cancel twice", async () => {
    open({ op: "cancel-scheduled", entity: "invoice-generate", sequences: [20399, 20799] });
    const sheet = await screen.findByRole("dialog", { name: "Unschedule messages" });
    expect(within(sheet).getByRole("button", { name: "Unschedule 2 messages" })).toBeInTheDocument();
    expect(within(sheet).getAllByRole("button", { name: /^Cancel/ })).toHaveLength(1);
  });

  it("nothing selected is said in muted text, not as an error", async () => {
    open({ op: "settle-deferred", entity: "invoice-generate", subQueue: "active", sequences: [], session: null });
    const sheet = await screen.findByRole("dialog", { name: "Settle deferred messages" });
    expect(within(sheet).getByText("Select the deferred messages to settle first.")).toHaveClass("text-muted");
    expect(sheet.querySelector(".text-accent")).toBeNull();
  });

  it("the target picker says when the namespace can't be listed and lets the name be typed", async () => {
    const { mock } = open({ op: "move", entity: "customs-docs", view: "active", sequences: [1], filter: {} }, "network");
    const sheet = await screen.findByRole("dialog", { name: "Move messages" });
    // A network error is retried once (about a second) before it shows: wait past the default 1 s.
    expect(await within(sheet).findByText("Couldn't list the queues and topics.", undefined, { timeout: 5000 })).toBeInTheDocument();
    await typeInto(within(sheet).getByRole("textbox", { name: "Entity name" }), "orders");
    await userEvent.click(within(sheet).getByRole("button", { name: "Move 1 message" }));
    await waitFor(() => expect(lastCall(mock, "op_move")).toMatchObject({ target: "orders" }));
    expect(within(sheet).queryByRole("region")).toBeNull();
  });

  it("the picker's filter is named for what it filters", async () => {
    open({ op: "move", entity: "customs-docs", view: "active", sequences: [1], filter: {} });
    const sheet = await screen.findByRole("dialog", { name: "Move messages" });
    expect(within(sheet).getByRole("textbox", { name: "Filter queues and topics" })).toBeInTheDocument();
    expect(within(sheet).getByRole("group", { name: "Move to" })).toBeInTheDocument();
  });

  it("a clone whose body could not be copied says so above the body and needs a body to send", async () => {
    const binary: MessageDetail = {
      sequence_number: 7, state: "active", message_id: "m-7", correlation_id: null, session_id: null, subject: null, content_type: null, to: null, reply_to: null,
      reply_to_session_id: null, partition_key: null, expires_at: null, enqueued_time: null, scheduled_enqueue_time: null, delivery_count: 0, time_to_live_ms: null,
      dead_letter_reason: null, dead_letter_description: null, dead_letter_source: null, locked_until: null, application_properties: {},
      body: { kind: "binary", value: btoa(String.fromCharCode(0xff, 0xfe, 0x00)) }, body_size: 3, body_truncated: false,
    };
    const { mock } = open({ op: "send", entity: "label-print", clone: binary });
    const sheet = await screen.findByRole("dialog", { name: "Send a copy" });
    const note = within(sheet).getByText(/The body is not text, so it was not copied/);
    expect(note).toHaveClass("text-12");
    expect(note.compareDocumentPosition(within(sheet).getByRole("textbox", { name: "Body" })) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(sheet).getByRole("button", { name: "Send 1 message" })).toBeDisabled();
    await typeInto(within(sheet).getByRole("textbox", { name: "Body" }), "x");
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(lastCall<{ form: SendForm }>(mock, "op_send")?.form.body).toEqual({ type: "text", text: "x" }));
  });

  it("optional properties sit behind a disclosure that counts what is set, and content type is a choice", async () => {
    const { mock } = open({ op: "send", entity: "label-print" });
    const sheet = await screen.findByRole("dialog", { name: "Send messages" });
    expect(within(sheet).queryByRole("textbox", { name: "Subject" })).toBeNull();
    const toggle = within(sheet).getByRole("button", { name: /^Properties/ });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(toggle);
    await typeInto(within(sheet).getByRole("textbox", { name: "Subject" }), "s");
    await userEvent.click(within(sheet).getByRole("radio", { name: "JSON" }));
    await userEvent.click(toggle);
    expect(toggle).toHaveTextContent("Properties (2 set)");
    expect(sheet.querySelector("datalist")).toBeNull();
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(lastCall<{ form: SendForm }>(mock, "op_send")?.form.template).toMatchObject({ subject: "s", content_type: "application/json" }));
  });

  it("the theme sets the colour scheme, so native pickers follow it", () => {
    const root = document.createElement("div");
    applyTheme("dark", "compact", root);
    expect(root.style.colorScheme).toBe("dark");
    applyTheme("light", "compact", root);
    expect(root.style.colorScheme).toBe("light");
  });

  it("session state: Replace is disabled for an empty value, and the lookup reads a state", async () => {
    open({ op: "session-state", entity: "returns-intake", session: "" });
    const sheet = await screen.findByRole("dialog", { name: "Session state" });
    expect(within(sheet).getByText("Choose a session to read its state.")).toBeInTheDocument();
    await typeInto(within(sheet).getByRole("textbox", { name: "Session" }), "RMA-1001");
    await userEvent.click(within(sheet).getByRole("button", { name: "Read state" }));
    expect(await within(sheet).findByText(/"step":"inspect"/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Replace state" })).toBeDisabled();
    expect(within(sheet).getByText("To remove the state, choose Clear.")).toBeInTheDocument();
  });

  it("purge names the scope in words, says how counts expire and what to do about an unknown one", async () => {
    open({ op: "purge", entities: ["stock-levels/subscriptions/replenish"] });
    const sheet = await screen.findByRole("dialog", { name: "Purge messages" });
    await userEvent.click(within(sheet).getByRole("radio", { name: "Transfer DLQ" }));
    expect(within(sheet).getByText(/Only the transfer dead-letter messages\./)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Count messages" }));
    expect(await within(sheet).findByText(/Refresh the tree and try again/)).toBeInTheDocument();
    expect(within(sheet).getByText("Counts expire after 10 minutes or one purge. If the sheet stays open, count again.")).toBeInTheDocument();
    expect(within(sheet).getByRole("group", { name: "Plan for stock-levels/subscriptions/replenish" })).toBeInTheDocument();
  });

  it("property errors attach to the control they are about", async () => {
    open({ op: "send", entity: "label-print" });
    const sheet = await screen.findByRole("dialog", { name: "Send messages" });
    for (let i = 0; i < 2; i++) await userEvent.click(within(sheet).getByRole("button", { name: "Add property" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Property 1 name" }), "a");
    await typeInto(within(sheet).getByRole("textbox", { name: "Property 2 name" }), "a");
    expect(within(sheet).getByRole("textbox", { name: "Property 2 name" })).toHaveAttribute("aria-invalid", "true");
    expect(within(sheet).getByRole("textbox", { name: "Property 2 value" })).not.toHaveAttribute("aria-invalid");
    act(() => {});
  });
});
