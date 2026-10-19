import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EngineError, commands, createQueryClient, setTransport, type IpcTransport, type MessageDetail } from "@/engine";
import { createMockTransport, type Scenario } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { DetailPane } from "./DetailPane";
import { bodyBytes, bodyText, decodeBody, toExportJson } from "./exportJson";

type Props = Partial<Parameters<typeof DetailPane>[0]>;

/** The mock, with `edit` able to change or hold the answer to a command. */
function around(edit: (cmd: string, args: Record<string, unknown> | undefined, answer: () => Promise<unknown>) => Promise<unknown>, scenario: Scenario = "default") {
  const mock = createMockTransport(scenario, 0);
  setTransport({ listen: mock.listen, invoke: <T,>(cmd: string, args?: Record<string, unknown>) => edit(cmd, args, () => mock.invoke(cmd, args)) as Promise<T> } satisfies IpcTransport);
}

async function pane(entity: string, view: "dead-letter" | "active", sequence: number | null, props: Props = {}, keepTransport = false) {
  if (!keepTransport) setTransport(createMockTransport("default", 0));
  await commands.connect("p-logistics");
  const onShowColumn = vi.fn();
  const onClose = vi.fn();
  const ui = (p: Props) => (
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <DetailPane profileId="p-logistics" entity={entity} view={view} sequence={sequence} onClose={onClose} onShowColumn={onShowColumn} {...p} />
      </TooltipProvider>
    </QueryClientProvider>
  );
  const client = createQueryClient();
  const utils = render(ui(props));
  return { onShowColumn, onClose, rerender: (p: Props) => utils.rerender(ui(p)) };
}

const firstOf = async (entity: string, view: "dead-letter" | "active", max: number, pick: (m: Awaited<ReturnType<typeof commands.peekPage>>["messages"][number]) => boolean) => {
  setTransport(createMockTransport("default", 0));
  await commands.connect("p-logistics");
  const page = await commands.peekPage("p-logistics", { entity, view, from: { kind: "start" }, max, filter: {} });
  return page.messages.find(pick)!;
};

afterEach(() => vi.restoreAllMocks());

describe("detail pane", () => {
  it("Clone & resend hands the loaded message to the composer", async () => {
    const m = await firstOf("customs-docs", "active", 5, () => true);
    const onClone = vi.fn();
    await pane("customs-docs", "active", m.sequence_number, { onClone });
    const region = await screen.findByRole("region", { name: `Message ${m.sequence_number}` });
    await within(region).findByRole("tab", { name: "Body" });
    await userEvent.click(within(region).getByRole("button", { name: /Clone & resend/ }));
    expect(onClone).toHaveBeenCalledWith(expect.objectContaining({ sequence_number: m.sequence_number, body: expect.anything() }));
  });

  it("shows the dead-letter reason and a formatted JSON body", async () => {
    const m = await firstOf("invoice-generate", "dead-letter", 37, (x) => x.body_preview?.startsWith("{") === true);
    await pane("invoice-generate", "dead-letter", m.sequence_number);
    const region = await screen.findByRole("region", { name: `Message ${m.sequence_number}` });
    expect(await within(region).findByText(m.dead_letter_reason!)).toBeInTheDocument();
    const body = await within(region).findByRole("region", { name: "Body" });
    expect(body.textContent).toContain('"shipmentId": "SHP-');
  });

  it("keeps big integers exact in the formatted body", async () => {
    const json = await firstOf("customs-docs", "active", 112, (m) => m.body_preview?.includes("trackingNumber") === true);
    await pane("customs-docs", "active", json.sequence_number);
    const body = await screen.findByRole("region", { name: "Body" });
    expect(body.textContent).toContain('"trackingNumber": 12345678901234567890');
  });

  it("copies the raw text, not the formatted view", async () => {
    const json = await firstOf("customs-docs", "active", 112, (m) => m.body_preview?.includes("trackingNumber") === true);
    const d = (await commands.messageDetail("p-logistics", "customs-docs", "active", json.sequence_number))!;
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    await pane("customs-docs", "active", json.sequence_number);
    await userEvent.click(await screen.findByRole("button", { name: "Copy body" }));
    expect(write).toHaveBeenCalledWith(bodyText(d.body));
    expect(write.mock.calls[0][0]).not.toContain("\n");
    expect(await screen.findByText("Copied body")).toBeInTheDocument();
  });

  it("binary body offers hex and no text copy", async () => {
    const bin = await firstOf("customs-docs", "active", 112, (m) => m.body_kind === "binary" && m.body_preview === null);
    await pane("customs-docs", "active", bin.sequence_number);
    const region = await screen.findByRole("region", { name: `Message ${bin.sequence_number}` });
    expect(await within(region).findByRole("region", { name: "Hex" })).toBeInTheDocument();
    expect(within(region).queryByRole("radio", { name: "Formatted" })).toBeNull();
    expect(within(region).getByText("Binary, not UTF-8 · 8 B")).toBeInTheDocument();
    expect(within(region).getByRole("button", { name: "Copy body as base64" })).toBeEnabled();
  });

  it("gone message shows the gone state, with a way to peek again", async () => {
    const onPeekAgain = vi.fn();
    await pane("invoice-generate", "dead-letter", 1, { onPeekAgain });
    expect(await screen.findByRole("heading", { name: "This message is no longer in the dead-letter queue" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Peek again" }));
    expect(onPeekAgain).toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Copy JSON" })).toHaveAttribute("aria-disabled", "true");
  });

  it("the message-gone scenario shows the gone state for a message the grid just listed", async () => {
    setTransport(createMockTransport("message-gone", 0));
    await pane("invoice-generate", "dead-letter", 41000, {}, true);
    expect(await screen.findByRole("heading", { name: "This message is no longer in the dead-letter queue" })).toBeInTheDocument();
  });

  it("with nothing selected it asks for a selection", async () => {
    await pane("invoice-generate", "dead-letter", null);
    expect(await screen.findByRole("heading", { name: "No message selected" })).toBeInTheDocument();
  });

  it("a truncated body says so, with the size actually shown, and cannot be copied", async () => {
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48213);
    expect(await screen.findByText("Showing the first 4.0 KiB of 5.0 MiB. Export the message for the whole body.")).toBeInTheDocument();
    expect(screen.getByText("Binary, shown as hex · 5.0 MiB")).toBeInTheDocument();
    // aria-disabled (not disabled), so the tooltip can say why; pressing does nothing.
    const write = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText: write }, configurable: true });
    const body = screen.getByRole("button", { name: "Copy body as base64" });
    const json = screen.getByRole("button", { name: "Copy JSON" });
    expect(body).toHaveAttribute("aria-disabled", "true");
    expect(json).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(body);
    await userEvent.click(json);
    expect(write).not.toHaveBeenCalled();
    act(() => {
      json.blur();
      json.focus();
    });
    expect(await screen.findByRole("tooltip")).toHaveTextContent("The body is cut at 4 MiB. Export the message to copy all of it.");
  });

  it("offers Export message beside the cut note only when the shell provides it", async () => {
    const onExport = vi.fn();
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48213, { onExport });
    await userEvent.click(await screen.findByRole("button", { name: "Export message" }));
    expect(onExport).toHaveBeenCalledWith(48213);
    expect(screen.queryByText(/Export the message for the whole body/)).toBeNull();
  });

  it("a cut log shows many rows of text and a multi-row hex view", async () => {
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48212);
    const body = await screen.findByRole("region", { name: "Body" });
    expect(body.querySelectorAll("[data-row]").length).toBeGreaterThan(5);
    expect(screen.getByText(/^Showing the first .* of 6\.0 MiB/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: "Hex" }));
    expect((await screen.findByRole("region", { name: "Hex" })).querySelectorAll("[data-row]").length).toBeGreaterThan(5);
  });

  it("the cut binary fixture has many hex rows", async () => {
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48213);
    expect((await screen.findByRole("region", { name: "Hex" })).querySelectorAll("[data-row]").length).toBeGreaterThan(10);
  });

  it("an empty body says so and has nothing to copy, but the message still copies", async () => {
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48211);
    expect(await screen.findByText("Empty body (0 bytes)")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Copy body/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Copy JSON" })).not.toHaveAttribute("aria-disabled");
  });

  it("nothing selected asks for a selection and cannot copy", async () => {
    await pane("invoice-generate", "dead-letter", null);
    expect(screen.getByRole("button", { name: "Copy JSON" })).toHaveAttribute("aria-disabled", "true");
  });

  it("lists custom properties with their types and can add one as a column", async () => {
    const { onShowColumn } = await pane("invoice-generate", "dead-letter", 41000);
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    const custom = screen.getByRole("table", { name: "Custom properties" });
    expect(within(custom).getByText("attempt")).toBeInTheDocument();
    expect(within(custom).getByText("int")).toBeInTheDocument();
    await userEvent.click(within(custom).getByRole("button", { name: "Show tenant as a column" }));
    expect(onShowColumn).toHaveBeenCalledWith("tenant");
  });

  it("follows the focused row after it rests, and keeps the chosen tab", async () => {
    const { rerender } = await pane("invoice-generate", "dead-letter", 41000);
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    rerender({ sequence: 41001 });
    expect(await screen.findByRole("region", { name: "Message 41001" })).toBeInTheDocument();
    // The content is the new message's; the old one is not shown meanwhile.
    await waitFor(() => expect(within(screen.getByRole("table", { name: "System properties" })).getByText("41001")).toBeInTheDocument());
    expect(screen.getByRole("tab", { name: "Properties" })).toHaveAttribute("aria-selected", "true");
  });

  it("Escape closes the pane", async () => {
    const { onClose } = await pane("invoice-generate", "dead-letter", 41000);
    await userEvent.click(await screen.findByRole("tab", { name: "Body" }));
    await userEvent.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalled();
  });

  it("lists system properties with absent ids as none, a readable time to live and no lock time", async () => {
    await pane("invoice-generate", "dead-letter", 41000);
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    const system = screen.getByRole("table", { name: "System properties" });
    const row = (k: string) => within(system).getByRole("rowheader", { name: k }).closest<HTMLElement>("[role=row]")!;
    expect(within(row("Session ID")).getByText("none")).toBeInTheDocument();
    expect(within(row("Time to live")).getByText("14 d")).toHaveAttribute("title", "1209600000 ms");
    expect(within(row("Expires at")).queryByText("none")).toBeNull();
    expect(within(system).queryByRole("rowheader", { name: "Locked until" })).toBeNull();
    // The reason is the header's, not a row.
    expect(within(system).queryByRole("rowheader", { name: "Dead-letter reason" })).toBeNull();
  });

  it("does not list the dead-letter properties again, and names the source in the header", async () => {
    const d = (await (async () => {
      setTransport(createMockTransport("default", 0));
      await commands.connect("p-logistics");
      return commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", 41000);
    })())!;
    around(async (cmd, _a, answer) => (cmd === "message_detail" ? { ...d, dead_letter_source: "invoice-generate" } : answer()));
    await pane("invoice-generate", "dead-letter", 41000, {}, true);
    expect(await screen.findByText("invoice-generate")).toBeInTheDocument();
    expect(screen.getByText(/Dead-lettered from/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "Properties" }));
    const custom = screen.getByRole("table", { name: "Custom properties" });
    expect(within(custom).queryByText("DeadLetterReason")).toBeNull();
    expect(within(custom).queryByText("DeadLetterErrorDescription")).toBeNull();
    expect(within(custom).getByText("tenant")).toBeInTheDocument();
  });

  it("puts the type after the value and hides the column button for a column already shown", async () => {
    await pane("invoice-generate", "dead-letter", 41000, { shownProps: ["tenant"] });
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    const custom = screen.getByRole("table", { name: "Custom properties" });
    const attempt = within(custom).getByText("attempt").closest<HTMLElement>("[role=row]")!;
    const cells = within(attempt).getAllByRole("cell");
    expect(cells).toHaveLength(1);
    expect(cells[0].textContent).toMatch(/^\d+int/);
    expect(within(custom).queryByRole("button", { name: "Show tenant as a column" })).toBeNull();
    expect(within(custom).getByRole("button", { name: "Show attempt as a column" })).toBeInTheDocument();
  });

  it("never shows the message that was selected before while the next one loads", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    around(async (cmd, args, answer) => {
      if (cmd === "message_detail" && args?.sequence === 41001) await gate;
      return answer();
    });
    const first = await (async () => {
      const { rerender } = await pane("invoice-generate", "dead-letter", 41000, {}, true);
      return rerender;
    })();
    expect(await screen.findByRole("tab", { name: "Body" })).toBeInTheDocument();
    const reasonOf = (await commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", 41000))!.dead_letter_reason!;
    expect(screen.getByRole("heading", { name: reasonOf })).toBeInTheDocument();
    first({ sequence: 41001 });
    // At once (before the focus has rested) and while the read is held: the pane has no old content.
    expect(screen.getByRole("region", { name: "Message 41001" })).toBeInTheDocument();
    expect(screen.queryByRole("tab", { name: "Body" })).toBeNull();
    await new Promise((r) => setTimeout(r, 200));
    expect(screen.queryByRole("tab", { name: "Body" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Body" })).toBeNull();
    expect(screen.getByRole("status", { name: "Loading" })).toBeInTheDocument();
    release();
    expect(await screen.findByRole("tab", { name: "Body" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Copy JSON" })).not.toHaveAttribute("aria-disabled");
  });

  it("shows a read that failed with its detail, and Try again reads it again", async () => {
    let fail = true;
    around(async (cmd, _a, answer) => {
      if (cmd === "message_detail" && fail) throw new EngineError("forbidden", "access denied (needs Listen): Unauthorized access.");
      return answer();
    });
    await pane("invoice-generate", "dead-letter", 41000, {}, true);
    expect(await screen.findByRole("heading", { name: "Couldn't read this message" })).toBeInTheDocument();
    expect(screen.getByText("access denied (needs Listen): Unauthorized access.")).toBeInTheDocument();
    fail = false;
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByRole("tab", { name: "Body" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Couldn't read this message" })).toBeNull();
  });

  it("says when JSON is too deep to indent and shows it as it came", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const d = (await commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", 41000))!;
    const deep = "[".repeat(1000) + "]".repeat(1000);
    around(async (cmd, _a, answer) => (cmd === "message_detail" ? { ...d, body: { kind: "text", value: deep }, body_size: deep.length } : answer()));
    await pane("invoice-generate", "dead-letter", 41000, {}, true);
    expect(await screen.findByText("Too large to format; showing raw text.")).toBeInTheDocument();
    expect((await screen.findByRole("region", { name: "Body" })).textContent).toContain(deep);
    expect(screen.queryByRole("radio", { name: "Formatted" })).toBeNull();
  });

  it("gives the System and Custom tables one key column, so their values line up", async () => {
    await pane("invoice-generate", "dead-letter", 41000);
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    const columns = (name: string) => screen.getByRole("table", { name }).style.gridTemplateColumns;
    expect(columns("System properties")).toBe(columns("Custom properties"));
    expect(columns("System properties")).toContain("min(");
  });

  it("has a heading for the message and for each section", async () => {
    await pane("invoice-generate", "dead-letter", 41000);
    expect(await screen.findByRole("heading", { level: 2, name: /Seq 41000/ })).toBeInTheDocument();
    await userEvent.click(await screen.findByRole("tab", { name: "Properties" }));
    expect(screen.getByRole("heading", { level: 3, name: "System" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 3, name: "Custom" })).toBeInTheDocument();
  });

  it("copy as JSON matches the export file's shape", async () => {
    setTransport(createMockTransport("default", 0));
    await commands.connect("p-logistics");
    const d = (await commands.messageDetail("p-logistics", "invoice-generate", "dead-letter", 41000))!;
    const json = JSON.parse(toExportJson(d));
    expect(Object.keys(json)[0]).toBe("schema");
    expect(json.schema).toBe("dispatched.message/v1");
    expect(json.sequence_number).toBe(41000);
    expect(json).not.toHaveProperty("body_truncated");
    expect(json).not.toHaveProperty("body_size");
    if (d.body.kind === "binary" && json.body_text !== undefined) expect(typeof json.body_text).toBe("string");
  });
});

describe("body helpers", () => {
  const detail = (body: MessageDetail["body"]) => ({ body }) as MessageDetail;
  it("decodes a body once and reads a cut UTF-8 body as text", () => {
    const utf8 = new TextEncoder().encode("héllo wörld");
    const cut = utf8.subarray(0, utf8.length - 1); // drops the last character, "d": still whole UTF-8
    const mid = new TextEncoder().encode("ab\u00e9").subarray(0, 3); // 61 62 c3: half of é
    const b64 = (u: Uint8Array) => ({ kind: "binary" as const, value: btoa(String.fromCharCode(...u)) });
    expect(decodeBody(b64(cut)).text).toBe("héllo wörl");
    expect(decodeBody(b64(mid)).text).toBeNull();
    expect(decodeBody(b64(mid), true).text).toBe("ab");
    expect(decodeBody(b64(mid), true).bytes).toHaveLength(3);
    // Really not UTF-8: still not text, cut or not.
    expect(decodeBody(b64(Uint8Array.of(0xff, 0xfe, 0x00)), true).text).toBeNull();
    expect(decodeBody({ kind: "binary", value: "***" }, true)).toEqual({ bytes: null, text: null });
  });
  it("decodes UTF-8 binary bodies and leaves others without text", () => {
    const utf8 = String.fromCharCode(...new TextEncoder().encode("héllo"));
    expect(bodyText({ kind: "binary", value: btoa(utf8) })).toBe("héllo");
    expect(bodyText({ kind: "binary", value: btoa("plain") })).toBe("plain");
    expect(bodyText({ kind: "binary", value: btoa(String.fromCharCode(0xff, 0xfe)) })).toBeNull();
    expect(bodyText({ kind: "binary", value: "***" })).toBeNull();
    expect(bodyBytes({ kind: "binary", value: "***" })).toBeNull();
    expect(bodyText({ kind: "value", value: { a: 1 } })).toBe('{"a":1}');
    expect(bodyText({ kind: "empty" })).toBe("");
    expect(bodyBytes({ kind: "sequence", value: [1] })).toBeNull();
    expect(toExportJson({ ...detail({ kind: "binary", value: btoa(String.fromCharCode(0xff)) }), sequence_number: 1 } as MessageDetail)).not.toContain("body_text");
  });
});
