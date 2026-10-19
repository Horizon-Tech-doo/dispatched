import { afterEach, describe, expect, it, vi } from "vitest";
import { commands, onActivityRecord, onConfirm, onJob, setTransport, type ActivityRecord, type ConfirmRequest, type JobEvent } from "@/engine";
import { createMockTransport, type Scenario } from "./transport";

type Finished = Extract<JobEvent, { type: "finished" }>;

/** A mock connected to the production profile, with every confirm, job event and activity record it emits. */
async function connected(scenario: Scenario) {
  const mock = createMockTransport(scenario);
  setTransport(mock);
  await commands.connect("p-logistics");
  const confirms: ConfirmRequest[] = [];
  const events: JobEvent[] = [];
  const records: ActivityRecord[] = [];
  const offs = await Promise.all([onConfirm((c) => confirms.push(c)), onJob((e) => events.push(e)), onActivityRecord((r) => records.push(r))]);
  const finishedOf = (id: string) => events.find((e): e is Finished => e.type === "finished" && e.id === id);
  return { mock, confirms, events, records, finishedOf, off: () => offs.forEach((f) => f()) };
}

const dlq = (sequences: string[]) => ({ entity: "invoice-generate", view: "dead-letter" as const, selection: { type: "sequences" as const, sequences } });

describe("operate mock", () => {
  afterEach(() => vi.useRealTimers());

  it("operate-prod asks a move twice, source then target, and finishes only after both answers", async () => {
    const t = await connected("operate-prod");
    const { id } = await commands.opMove("p-logistics", dlq(["41000", "41001"]), "label-print", false);
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    expect(t.confirms[0]).toMatchObject({ job_id: id, target: "invoice-generate", production: true, step: 1, verb: "move" });
    expect(t.confirms[0].summary).toMatch(/^Move 2 messages from invoice-generate\/\$DeadLetterQueue to label-print on Logistics/);
    await commands.confirmAnswer(t.confirms[0].id, { type: "typed", text: "invoice-generate" });
    await vi.waitFor(() => expect(t.confirms).toHaveLength(2));
    expect(t.confirms[1]).toMatchObject({ job_id: id, target: "label-print", step: 2 });
    await new Promise((r) => setTimeout(r, 5));
    expect(t.finishedOf(id)).toBeUndefined();
    await commands.confirmAnswer(t.confirms[1].id, { type: "typed", text: "label-print" });
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    expect(t.finishedOf(id)).toMatchObject({ error: null, report: { outcome: "succeeded", succeeded: 2, target: "label-print" } });
    t.off();
  });

  it("operate-prod asks a delete once, and only the typed name lets it run", async () => {
    const t = await connected("operate-prod");
    const { id } = await commands.opDelete("p-logistics", dlq(["41000"]));
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    expect(t.confirms[0]).toMatchObject({ verb: "delete", target: "invoice-generate", step: 1, item_count: "1" });
    // Allow alone is not enough on production.
    await commands.confirmAnswer(t.confirms[0].id, { type: "allow" });
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    expect(t.finishedOf(id)!.error).toMatchObject({ kind: "denied" });
    expect(t.confirms).toHaveLength(1);
    expect(t.records.at(-1)).toMatchObject({ origin: "gui", phase: "finished", outcome: "denied" });
    t.off();
  });

  it("a cancelled job answers its waiting confirmation no and ends cancelled with nothing changed", async () => {
    const t = await connected("operate-ok");
    const { id } = await commands.opDelete("p-logistics", dlq(["41000"]));
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    expect(t.confirms[0].production).toBe(false);
    expect(await commands.jobCancel(id)).toBe(true);
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    expect(t.finishedOf(id)!.error).toMatchObject({ kind: "cancelled" });
    expect(await commands.confirmAnswer(t.confirms[0].id, { type: "allow" })).toBe(false);
    const page = await commands.peekPage("p-logistics", { entity: "invoice-generate", view: "dead-letter", from: { kind: "start" }, max: 1, filter: {} });
    expect(page.messages[0].sequence_number).toBe(41000);
  });

  it("emits items, records started and finished as gui, and takes deleted messages away", async () => {
    const t = await connected("operate-ok");
    const { id } = await commands.opDelete("p-logistics", dlq(["41000", "41001", "99"]));
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    await commands.confirmAnswer(t.confirms[0].id, { type: "allow" });
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    const items = t.events.filter((e) => e.type === "item" && e.id === id);
    expect(items.map((e) => (e as Extract<JobEvent, { type: "item" }>).item.outcome)).toEqual(["succeeded", "succeeded", "missing"]);
    expect(t.finishedOf(id)!.report).toMatchObject({ succeeded: 2, missing: 1, incomplete: 1 });
    expect(t.records.map((r) => [r.phase, r.outcome, r.origin])).toEqual([["started", "pending", "gui"], ["finished", "succeeded", "gui"]]);
    const page = await commands.peekPage("p-logistics", { entity: "invoice-generate", view: "dead-letter", from: { kind: "start" }, max: 1, filter: {} });
    expect(page.messages[0].sequence_number).toBe(41002);
    const report = await commands.jobReport(id, 0, 100, true);
    expect(report?.items).toEqual([expect.objectContaining({ sequence_number: "99", outcome: "missing" })]);
    const since = await commands.activitySince(null);
    expect(since).toHaveLength(1);
    expect(since[0].id).toBe(t.records.at(-1)!.id);
  });

  it("operate-partial fails the second message and keeps the failed count", async () => {
    const t = await connected("operate-partial");
    const { id } = await commands.opDelete("p-logistics", dlq(["41000", "41001", "41002"]));
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    await commands.confirmAnswer(t.confirms[0].id, { type: "allow" });
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    expect(t.finishedOf(id)!.report).toMatchObject({ outcome: "partly-failed", succeeded: 2, failed: 1 });
  });

  it("operate-stopped ends stopped and keeps the items so far", async () => {
    const t = await connected("operate-stopped");
    const { id } = await commands.opDelete("p-logistics", dlq(["41000", "41001", "41002"]));
    await vi.waitFor(() => expect(t.confirms).toHaveLength(1));
    await commands.confirmAnswer(t.confirms[0].id, { type: "allow" });
    await vi.waitFor(() => expect(t.finishedOf(id)).toBeDefined());
    expect(t.finishedOf(id)).toMatchObject({ error: { kind: "stopped" }, report: { cancelled: true, succeeded: 1, items: 1 } });
  });
});
