import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { commands, createQueryClient, setTransport, type MessageDetail } from "@/engine";
import { createMockTransport } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { DetailPane } from "./DetailPane";
import { bodyBytes, bodyText, toExportJson } from "./exportJson";

type Props = Partial<Parameters<typeof DetailPane>[0]>;

async function pane(entity: string, view: "dead-letter" | "active", sequence: number | null, props: Props = {}) {
  setTransport(createMockTransport("default", 0));
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
    expect(within(region).getByText("Not valid UTF-8, shown as hex")).toBeInTheDocument();
    expect(within(region).getByRole("button", { name: "Copy body as base64" })).toBeEnabled();
  });

  it("gone message shows the gone state", async () => {
    await pane("invoice-generate", "dead-letter", 1);
    expect(await screen.findByRole("heading", { name: "This message is no longer in the dead-letter queue" })).toBeInTheDocument();
  });

  it("with nothing selected it asks for a selection", async () => {
    await pane("invoice-generate", "dead-letter", null);
    expect(await screen.findByRole("heading", { name: "No message selected" })).toBeInTheDocument();
  });

  it("a truncated body says so and disables copying", async () => {
    await pane("shipment-events/subscriptions/tracking-sync", "dead-letter", 48213);
    expect(await screen.findByText("Showing the first 4.0 MiB of 5.0 MiB. Export the message for the whole body.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Copy body/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Copy as JSON" })).toBeDisabled();
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
