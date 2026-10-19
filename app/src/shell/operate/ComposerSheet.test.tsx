import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { MessageDetail, SendForm } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall, typeInto } from "./testing";

const message: MessageDetail = {
  sequence_number: 41000, state: "active", message_id: "order-0009", correlation_id: "corr-1", session_id: null, subject: "order.created", content_type: "application/json",
  to: null, reply_to: "replies", reply_to_session_id: null, partition_key: null, expires_at: null, enqueued_time: "2026-10-15T09:00:00Z", scheduled_enqueue_time: null,
  delivery_count: 1, time_to_live_ms: 90_000, dead_letter_reason: null, dead_letter_description: null, dead_letter_source: null, locked_until: null,
  application_properties: {
    tenant: { type: "string", value: "contoso" },
    attempt: { type: "int", value: 3 },
    big: { type: "long", value: 12345 },
    legacy: { type: "other", value: "described(…)" },
  },
  body: { kind: "text", value: '{"orderId":9}' }, body_size: 13, body_truncated: false,
};

async function composer(seed: { entity?: string | null; clone?: MessageDetail | null } = {}) {
  const utils = renderShell("operate-ok", { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", op: "send", entity: seed.entity ?? "label-print", clone: seed.clone ?? null }} />);
  const sheet = await screen.findByRole("dialog", { name: seed.clone ? "Send a copy" : "Send messages" });
  return { sheet, ...utils };
}

const sent = (mock: Parameters<typeof lastCall>[0]) => lastCall<{ form: SendForm }>(mock, "op_send")?.form;

describe("ComposerSheet", () => {
  it("auto-increment previews order-0009 → order-0010", async () => {
    const { sheet } = await composer();
    await userEvent.click(within(sheet).getByRole("radio", { name: "Increment" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "First message ID" }), "order-0009");
    fireEvent.change(within(sheet).getByRole("spinbutton", { name: "Count" }), { target: { value: "2" } });
    expect(within(sheet).getByText(/order-0009 → order-0010/)).toBeInTheDocument();
  });

  it("sends a text body with its properties and the typed ids", async () => {
    const { sheet, mock } = await composer();
    await typeInto(within(sheet).getByRole("textbox", { name: "Body" }), "hello");
    await userEvent.click(within(sheet).getByRole("button", { name: /^Properties/ }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Subject" }), "greeting");
    await typeInto(within(sheet).getByRole("textbox", { name: "Time to live" }), "5m");
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(sent(mock)).toBeDefined());
    expect(sent(mock)).toMatchObject({ target: "label-print", body: { type: "text", text: "hello" }, ids: { type: "random" }, count: 1, batch: false, template: { subject: "greeting", time_to_live: "5m", properties: [] } });
    expect(sent(mock)!.schedule).toBeUndefined();
    // The confirmation follows in the same sheet.
    expect(await within(sheet).findByText(/^Send 1 message to label-print on Logistics/)).toBeInTheDocument();
  });

  it("an invalid time to live blocks sending", async () => {
    const { sheet } = await composer();
    await userEvent.click(within(sheet).getByRole("button", { name: /^Properties/ }));
    const ttl = within(sheet).getByRole("textbox", { name: "Time to live" });
    await typeInto(ttl, "soon");
    expect(ttl).toHaveAttribute("aria-invalid", "true");
    expect(within(sheet).getByRole("button", { name: "Send 1 message" })).toBeDisabled();
  });

  it("clone seeds typed properties and marks \"other\" as not copyable", async () => {
    const { sheet, mock } = await composer({ entity: "label-print", clone: message });
    expect(within(sheet).getByRole("textbox", { name: "Body" })).toHaveValue('{"orderId":9}');
    expect(within(sheet).getByRole("textbox", { name: "Subject" })).toHaveValue("order.created");
    expect(within(sheet).getByRole("textbox", { name: "Reply to" })).toHaveValue("replies");
    expect(within(sheet).getByRole("textbox", { name: "Time to live" })).toHaveValue("90s");
    expect(within(sheet).getByRole("textbox", { name: "Message ID" })).toHaveValue("order-0009");
    const names = within(sheet).getAllByRole("textbox", { name: /^Property \d name$/ }).map((i) => (i as HTMLInputElement).value);
    expect(names).toEqual(["tenant", "attempt", "big"]);
    expect(within(sheet).getByRole("combobox", { name: "Property 2 type" })).toHaveTextContent("int");
    expect(within(sheet).getByRole("combobox", { name: "Property 3 type" })).toHaveTextContent("long");
    const legacy = within(sheet).getByRole("group", { name: "Property 4" });
    expect(within(legacy).getByText(/not copyable/)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(sent(mock)).toBeDefined());
    expect(sent(mock)!.ids).toEqual({ type: "fixed", id: "order-0009" });
    expect(sent(mock)!.template.properties).toEqual([
      { name: "tenant", type: "string", value: "contoso" },
      { name: "attempt", type: "int", value: "3" },
      { name: "big", type: "long", value: "12345" },
    ]);
  });

  it("edit sheet keeps 9007199254740993", async () => {
    const { sheet, mock } = await composer();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add property" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Property 1 name" }), "big");
    await userEvent.click(within(sheet).getByRole("combobox", { name: "Property 1 type" }));
    await userEvent.click(await screen.findByRole("option", { name: "long" }));
    await typeInto(within(sheet).getByRole("textbox", { name: "Property 1 value" }), "9007199254740993");
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(sent(mock)).toBeDefined());
    expect(sent(mock)!.template.properties).toEqual([{ name: "big", type: "long", value: "9007199254740993" }]);
  });

  it("schedule sends RFC3339 in UTC", async () => {
    const { sheet, mock } = await composer();
    fireEvent.change(within(sheet).getByLabelText("Schedule for"), { target: { value: "2026-10-20T14:30" } });
    await userEvent.click(within(sheet).getByRole("button", { name: "Schedule 1 message" }));
    await waitFor(() => expect(sent(mock)).toBeDefined());
    const when = sent(mock)!.schedule!;
    expect(when).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/);
    expect(Date.parse(when)).toBe(new Date(2026, 9, 20, 14, 30).getTime());
  });

  it("files body needs at least one pick", async () => {
    const { sheet, mock } = await composer();
    await userEvent.click(within(sheet).getByRole("radio", { name: "Files" }));
    expect(within(sheet).getByText("Choose at least one file.")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Send 1 message" })).toBeDisabled();
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose files…" }));
    expect(await within(sheet).findByText("order-1001.json")).toBeInTheDocument();
    expect(within(sheet).queryByText("Choose at least one file.")).toBeNull();
    await userEvent.click(within(sheet).getByRole("button", { name: "Send 1 message" }));
    await waitFor(() => expect(sent(mock)).toBeDefined());
    expect(sent(mock)!.body).toEqual({ type: "files", pick: expect.stringMatching(/^open-/) });
  });
});

describe("ImportSheet", () => {
  it("imports a picked export file into the entity", async () => {
    const { mock } = renderShell("operate-ok", { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", op: "import", entity: "label-print" }} />);
    const sheet = await screen.findByRole("dialog", { name: "Import messages" });
    expect(within(sheet).getByRole("button", { name: "Import messages" })).toBeDisabled();
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file…" }));
    expect(await within(sheet).findByText("orders-dead-letter-20261016-0941.json")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Import messages" }));
    await waitFor(() => expect(lastCall(mock, "op_import")).toBeDefined());
    expect(lastCall(mock, "op_import")).toMatchObject({ form: { target: "label-print", new_message_id: false, keep_sessions: true, keep_dead_letter_properties: false } });
  });
});
