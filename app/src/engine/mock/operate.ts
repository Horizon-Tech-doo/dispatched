import { EngineError } from "../ipc";
import type {
  ActivityOrigin, ActivityOutcome, ActivityQuery, ActivityRecord, ConfirmAnswer, ConfirmRequest, DlqSummaryView, EntityKindName, FilterSpec, ImportForm,
  ItemOutcome, ItemView, JobEvent, JobKind, JobStarted, OpenedFiles, OpenPurpose, PeekView, PickedPath, PurgePlans, PurgePlanView, PurgeScope, ReceiveForm,
  ReceivedView, ReportPage, ReportSummary, ResubmitForm, SelectForm, SendForm, SessionStateView, Settlement, StoredProfile, SubQueue,
} from "../types";
import { accepts, summarize, type MockMessage } from "./messages";

/** What the operate mock needs from the transport. */
export interface OperateHost {
  scenario: string;
  /** Milliseconds between a job's steps (0 in tests). */
  tick: number;
  emit(event: string, payload: unknown): void;
  connectedTo(profileId: unknown): StoredProfile;
  /** Every message of an entity still there (all sub-queues). */
  messagesOf(entity: string): MockMessage[];
  /** Takes messages away (deleted, received, moved, purged). */
  remove(entity: string, sequences: number[]): void;
  matches(m: MockMessage, f: FilterSpec): boolean;
  /** The entity's kind as the namespace lists it, or null when it is not listed. */
  kindOf(entity: string): EntityKindName | null;
  /** The listed counts (null when unknown or not listed). */
  countsOf(entity: string): { active: number; dead_letter: number; transfer_dead_letter: number } | null;
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
const thousands = (n: number) => n.toLocaleString("en-US");
const messages = (n: number) => `${thousands(n)} ${n === 1 ? "message" : "messages"}`;
const VIEW_SUB: Record<PeekView, SubQueue> = {
  active: "active", scheduled: "active", deferred: "active", "dead-letter": "dead-letter", "transfer-dead-letter": "transfer-dead-letter",
};
const SUB_SUFFIX: Record<SubQueue, string> = { active: "", "dead-letter": "/$DeadLetterQueue", "transfer-dead-letter": "/$Transfer/$DeadLetterQueue" };
const subOf = (q: SubQueue): MockMessage["sub"] => (q === "dead-letter" ? "dlq" : q === "transfer-dead-letter" ? "tdlq" : "main");
const VERB: Record<string, string> = {
  send: "Sent", import: "Sent", "cancel-scheduled": "Cancelled", receive: "Received", "settle-deferred": "Settled", delete: "Deleted", move: "Moved",
  resubmit: "Resubmitted", purge: "Removed",
};

/** As core `JobReport::summary_line`. */
function summaryLine(kind: JobKind, entity: string, target: string | null, items: ItemView[], cancelled: boolean): string {
  const verb = VERB[kind] ?? "Done";
  const place = target ? `from ${entity} to ${target}` : kind === "send" || kind === "import" ? `to ${entity}` : `from ${entity}`;
  const n = (o: ItemOutcome) => items.filter((i) => i.outcome === o).length;
  const unreached = n("not-reached");
  const stop = !cancelled ? "" : unreached ? ` (cancelled; ${thousands(unreached)} not attempted)` : " (cancelled)";
  const others = ([[n("skipped"), "skipped"], [n("missing"), "missing"]] as const).filter(([c]) => c > 0).map(([c, w]) => `${thousands(c)} ${w}`);
  const rest = others.length ? `; ${others.join(", ")}` : "";
  const failed = items.filter((i) => i.outcome === "failed");
  if (!failed.length) return `${verb} ${n("succeeded")} ${n("succeeded") === 1 ? "message" : "messages"} ${place}${stop}${rest}`;
  const list = failed.slice(0, 5).map((i) => (i.sequence_number ? `sequence ${i.sequence_number}: ${i.reason}` : `message ${i.message_id}: ${i.reason}`));
  return `${verb} ${n("succeeded")} of ${items.length} messages ${place}${stop}; ${failed.length} failed (${list.join(", ")}${failed.length > 5 ? `, and ${failed.length - 5} more` : ""})${rest}`;
}

function reportOf(kind: JobKind, entity: string, target: string | null, items: ItemView[], notes: string[], cancelled: boolean, opId: string | null): ReportSummary {
  const n = (o: ItemOutcome) => items.filter((i) => i.outcome === o).length;
  const succeeded = n("succeeded");
  const failed = n("failed");
  return {
    op_id: opId, kind, entity, target,
    outcome: cancelled ? "cancelled" : failed === 0 ? "succeeded" : succeeded === 0 ? "failed" : "partly-failed",
    cancelled, items: items.length, succeeded, failed, skipped: n("skipped"), missing: n("missing"),
    incomplete: items.length - succeeded, notes, summary_line: summaryLine(kind, entity, target, items, cancelled),
  };
}

/** One request a job confirms: the core's sentence and the entity the typed name must equal. */
interface Ask { verb: string; target: string; summary: string; count: number | null; reversible: boolean }

/** A job's work, given the confirmations passed: the items in order, notes, and received messages. */
interface Work {
  items: ItemView[];
  notes?: string[];
  received?: ReceivedView[];
  /** Runs when the item at this index is reported (the change happens then). */
  apply?: (index: number) => void;
}

interface Running {
  cancelled: boolean;
  stopped: boolean;
  /** Answers the waiting confirmation "no". */
  deny?: () => void;
}

/** The activity log as `activity.db` holds it: a CLI purge and an agent's move before the app opened. */
function seedLog(): ActivityRecord[] {
  const base = Date.parse("2026-10-19T07:58:00Z");
  const rec = (i: number, origin: ActivityOrigin, phase: "started" | "finished", outcome: ActivityOutcome, action: string, entity: string, summary: string, count: number | null): ActivityRecord => ({
    id: i, op_id: `op-seed-${Math.ceil(i / 2)}`, at: new Date(base + i * 60_000).toISOString(), origin, profile: "Logistics dev", entity, action, kind: "mutating",
    phase, outcome, count, succeeded: phase === "finished" ? count : null, failed: phase === "finished" ? 0 : null, skipped: phase === "finished" ? 0 : null,
    summary, error: null, note: null,
  });
  return [
    rec(1, "cli", "started", "pending", "purge", "label-print", "Purge label-print (active) on Logistics dev", 6),
    rec(2, "cli", "finished", "succeeded", "purge", "label-print", "Removed 6 messages from label-print", 6),
    rec(3, "mcp", "started", "pending", "move", "returns-intake", "Move 2 messages from returns-intake/$DeadLetterQueue to returns-intake on Logistics dev", 2),
    rec(4, "mcp", "finished", "succeeded", "move", "returns-intake", "Moved 2 messages from returns-intake to returns-intake", 2),
  ];
}

export function createOperateMock(h: OperateHost) {
  let seq = 0;
  const next = (p: string) => `${p}-${++seq}`;
  const running = new Map<string, Running>();
  const confirms = new Map<string, { jobId: string; production: boolean; target: string; resolve: (ok: boolean) => void }>();
  const reports = new Map<string, { summary: ReportSummary; items: ItemView[] }>();
  const picks = new Map<string, string[]>();
  const savePicks = new Map<string, string>();
  const plans = new Map<string, { profileId: string; plans: PurgePlanView[]; at: number }>();
  const states = new Map<string, Uint8Array>();
  const log: ActivityRecord[] = seedLog();
  let nextNewSeq = 90_000;

  const job = (e: JobEvent) => h.emit("job", e);
  const production = (p: StoredProfile) => (h.scenario === "operate-prod" ? true : h.scenario.startsWith("operate-") ? false : p.production);

  /** Writes an entry to the shared log and emits it, as `EmittingAudit` does. */
  const record = (r: Omit<ActivityRecord, "id" | "at">): ActivityRecord => {
    const full: ActivityRecord = { ...r, id: (log.at(-1)?.id ?? 0) + 1, at: new Date().toISOString() };
    log.push(full);
    if (full.origin === "gui") h.emit("activity-record", full);
    return full;
  };

  /** Asks one question and waits for its answer: true to go ahead. */
  const ask = (jobId: string, a: Ask, prod: boolean, step: number) =>
    new Promise<boolean>((resolve) => {
      const id = next("confirm");
      confirms.set(id, { jobId, production: prod, target: a.target, resolve });
      const run = running.get(jobId);
      if (run) run.deny = () => {
        confirms.delete(id);
        resolve(false);
      };
      h.emit("confirm", {
        id, job_id: jobId, verb: a.verb, target: a.target, summary: a.summary, item_count: a.count === null ? null : String(a.count),
        reversible: a.reversible, production: prod, step,
      } satisfies ConfirmRequest);
    });

  /**
   * Starts a job the way `start_op` does: the id comes back at once, then `started`, the confirmations (each answered
   * before the next is asked), the items one by one, and `finished` with the report.
   */
  function start(profileId: unknown, o: { kind: JobKind; title: string; entity: string; target?: string | null; asks: Ask[]; work: () => Work }): JobStarted {
    const p = h.connectedTo(profileId);
    const prod = production(p);
    const id = next("job");
    const run: Running = { cancelled: false, stopped: false };
    running.set(id, run);
    const target = o.target ?? null;
    const finish = (error: { kind: string; message: string } | null, items: ItemView[] | null, notes: string[], opId: string | null) => {
      running.delete(id);
      const cancelled = error !== null;
      const summary = items && (items.length || !error) ? reportOf(o.kind, o.entity, target, items, notes, cancelled, opId) : null;
      if (summary && items) reports.set(id, { summary, items });
      job({ type: "finished", id, error, report: summary, summary: summary?.summary_line ?? null, output: null, written: null, missing: null, file_name: null });
    };
    void (async () => {
      await wait(0);
      job({ type: "started", id, kind: o.kind, title: o.title, total: null, file_name: null });
      const first = o.asks[0];
      const base = { origin: "gui" as const, profile: p.name, entity: o.entity, action: first.verb, kind: "mutating", count: first.count, note: null };
      // Off production one question; on production one per entity the operation changes (a purge asks per entity anyway).
      const asks = prod || o.kind === "purge" ? o.asks : o.asks.slice(0, 1);
      for (let i = 0; i < asks.length; i++) {
        const ok = run.cancelled || run.stopped ? false : await ask(id, asks[i], prod, i + 1);
        if (!ok) {
          const why = run.stopped ? "stopped" : run.cancelled ? "cancelled" : "denied";
          const opId = next("op");
          record({ ...base, op_id: opId, phase: "finished", outcome: "denied", succeeded: null, failed: null, skipped: null, summary: first.summary, error: why === "denied" ? "not confirmed" : "cancelled before it was confirmed" });
          finish(
            why === "stopped" ? { kind: "stopped", message: "Stopped: the connection closed." } : why === "cancelled" ? { kind: "cancelled", message: "Cancelled." } : { kind: "denied", message: "denied: not confirmed" },
            null, [], opId,
          );
          return;
        }
      }
      const opId = next("op");
      record({ ...base, op_id: opId, phase: "started", outcome: "pending", succeeded: null, failed: null, skipped: null, summary: first.summary, error: null });
      const w = o.work();
      const done: ItemView[] = [];
      for (const n of w.notes ?? []) job({ type: "note", id, text: n });
      for (let i = 0; i < w.items.length; i++) {
        await wait(h.tick);
        // `operate-stopped`: the connection closes after the first message.
        if (h.scenario === "operate-stopped" && i === 1) run.stopped = true;
        if (run.cancelled || run.stopped) {
          const error = run.stopped ? { kind: "stopped", message: "Stopped: the connection closed." } : { kind: "cancelled", message: "Cancelled." };
          const s = reportOf(o.kind, o.entity, target, done, w.notes ?? [], true, opId);
          record({ ...base, op_id: opId, phase: "finished", outcome: "cancelled", succeeded: s.succeeded, failed: s.failed, skipped: s.skipped, summary: s.summary_line, error: error.message });
          finish(error, done, w.notes ?? [], opId);
          return;
        }
        let item = w.items[i];
        // `operate-partial`: the second message fails.
        if (h.scenario === "operate-partial" && i === 1 && item.outcome === "succeeded")
          item = { ...item, outcome: "failed", reason: "lock lost: the message's lock expired before it was settled", new_sequence_number: null };
        if (item.outcome === "succeeded") w.apply?.(i);
        const rec = w.received?.[i];
        if (rec) job({ type: "received", id, message: rec });
        done.push(item);
        job({ type: "item", id, item });
        job({ type: "progress", id, done: done.length, total: w.items.length, scanned: done.length });
      }
      const s = reportOf(o.kind, o.entity, target, done, w.notes ?? [], false, opId);
      record({
        ...base, op_id: opId, phase: "finished", outcome: s.outcome, succeeded: s.succeeded, failed: s.failed, skipped: s.skipped, summary: s.summary_line, error: null,
        note: s.notes[0] ?? null,
      });
      finish(null, done, w.notes ?? [], opId);
    })();
    return { id };
  }

  const item = (entity: string, m: Partial<ItemView> & Pick<ItemView, "outcome">): ItemView => ({
    sequence_number: null, message_id: null, session_id: null, entity, new_sequence_number: null, reason: null, ...m,
  });

  /** The messages a selection names, and the items for the ones that were not there. */
  function select(f: SelectForm): { found: MockMessage[]; items: (m: MockMessage | string) => ItemView; missing: string[]; notes: string[]; count: number } {
    const inView = h.messagesOf(f.entity).filter((m) => accepts(f.view, m));
    const items = (m: MockMessage | string) =>
      typeof m === "string"
        ? item(f.entity, { sequence_number: m, outcome: "missing", reason: "no longer in the view" })
        : item(f.entity, { sequence_number: String(m.sequence_number), message_id: m.message_id, session_id: m.session_id, outcome: "succeeded" });
    if (f.selection.type === "sequences") {
      if (!f.selection.sequences.length) throw new EngineError("invalid", "Select the messages first.");
      const want = f.selection.sequences;
      const found = want.map((s) => inView.find((m) => String(m.sequence_number) === s)).filter((m): m is MockMessage => !!m);
      const missing = want.filter((s) => !found.some((m) => String(m.sequence_number) === s));
      return { found, items, missing, notes: [], count: want.length };
    }
    const sel = f.selection;
    const found = inView.filter((m) => h.matches(m, sel.filter)).slice(0, sel.limit);
    const notes = [`matched ${thousands(found.length)} in the first ${thousands(inView.length)} examined`];
    return { found, items, missing: [], notes, count: found.length };
  }

  const at = (entity: string, view: PeekView) => `${entity}${SUB_SUFFIX[VIEW_SUB[view]]}`;
  const name = (profileId: unknown) => h.connectedTo(profileId).name;

  function handle(cmd: string, args: Record<string, unknown>): unknown {
    switch (cmd) {
      case "confirm_answer": {
        const c = confirms.get(args.id as string);
        if (!c) return false;
        confirms.delete(args.id as string);
        const a = args.answer as ConfirmAnswer;
        // As the kit's `require`: production needs the typed name, and it must match.
        const ok = a.type === "allow" ? !c.production : a.type === "typed" ? a.text === c.target : false;
        c.resolve(ok);
        return true;
      }
      case "op_send": {
        const f = args.form as SendForm;
        if (!f.target?.trim()) throw new EngineError("invalid", "Choose where to send the messages.");
        if (!(f.count >= 1)) throw new EngineError("invalid", "Send at least one message.");
        if ((f.body.type === "files" || f.body.type === "template") && !picks.has(f.body.pick)) throw new EngineError("invalid", "Choose the file again.");
        if (f.body.type !== "text") picks.delete(f.body.pick);
        const on = name(args.profileId);
        const verb = f.schedule ? "schedule" : "send";
        const summary = f.schedule ? `Schedule ${messages(f.count)} to ${f.target} for ${f.schedule} on ${on}` : `Send ${messages(f.count)} to ${f.target} on ${on}`;
        return start(args.profileId, {
          kind: "send", title: `${f.schedule ? "Schedule" : "Send"} ${thousands(f.count)} to ${f.target}`, entity: f.target,
          asks: [{ verb, target: f.target, summary, count: f.count, reversible: false }],
          work: () => ({
            items: Array.from({ length: f.count }, (_, i) =>
              item(f.target, {
                message_id: f.ids.type === "fixed" ? f.ids.id : f.ids.type === "increment" ? `${f.ids.base}-${i}` : `mock-${nextNewSeq + i}`,
                new_sequence_number: String(nextNewSeq++), outcome: "succeeded",
              }),
            ),
          }),
        });
      }
      case "op_import": {
        const f = args.form as ImportForm;
        const files = picks.get(f.pick);
        if (!files) throw new EngineError("invalid", "Choose the file again.");
        picks.delete(f.pick);
        const n = 5;
        return start(args.profileId, {
          kind: "import", title: `Import ${n} into ${f.target}`, entity: f.target,
          asks: [{ verb: "import", target: f.target, summary: `Send ${n} imported messages to ${f.target} on ${name(args.profileId)}`, count: n, reversible: false }],
          work: () => ({ items: Array.from({ length: n }, (_, i) => item(f.target, { message_id: `imported-${i + 1}`, new_sequence_number: String(nextNewSeq++), outcome: "succeeded" })) }),
        });
      }
      case "op_cancel_scheduled": {
        const entity = args.entity as string;
        const sequences = args.sequences as string[];
        if (!sequences?.length) throw new EngineError("invalid", "Select the scheduled messages to cancel.");
        const sel = select({ entity, view: "scheduled", selection: { type: "sequences", sequences } });
        return start(args.profileId, {
          kind: "cancel-scheduled", title: `Cancel ${thousands(sequences.length)} scheduled in ${entity}`, entity,
          asks: [{ verb: "cancel-scheduled", target: entity, summary: `Cancel ${thousands(sequences.length)} scheduled ${sequences.length === 1 ? "message" : "messages"} in ${entity} on ${name(args.profileId)}`, count: sequences.length, reversible: false }],
          work: () => ({ items: [...sel.found.map(sel.items), ...sel.missing.map(sel.items)], apply: (i) => sel.found[i] && h.remove(entity, [sel.found[i].sequence_number]) }),
        });
      }
      case "op_receive": {
        const f = args.form as ReceiveForm;
        if (!(f.max >= 1 && f.max <= 5000)) throw new EngineError("invalid", "Receive between 1 and 5,000 messages.");
        const sub = subOf(f.sub_queue);
        const ms = h.messagesOf(f.entity).filter((m) => m.sub === sub && m.state === "active" && (!f.session || m.session_id === f.session)).slice(0, f.max);
        const address = `${f.entity}${SUB_SUFFIX[f.sub_queue]}`;
        const on = name(args.profileId);
        const del = f.action.type === "delete";
        const how: Settlement | null = f.action.type === "settle" ? f.action.how : null;
        const tail = how?.type === "complete" ? "complete them (removes them)" : how?.type === "abandon" ? "abandon them" : how?.type === "defer" ? "defer them" : "dead-letter them";
        const summary = del ? `Remove up to ${messages(f.max)} from ${address} on ${on} (receive and delete)` : `Receive up to ${messages(f.max)} from ${address} and ${tail} on ${on}`;
        const removes = del || how?.type === "complete" || how?.type === "dead-letter";
        return start(args.profileId, {
          kind: "receive", title: `Receive ${del ? "and delete" : "and settle"} up to ${f.max} from ${f.entity}`, entity: f.entity,
          asks: [{ verb: del ? "receive-and-delete" : (how?.type ?? "complete"), target: f.entity, summary, count: f.max, reversible: !removes }],
          work: () => ({
            items: ms.map((m) => item(f.entity, { sequence_number: String(m.sequence_number), message_id: m.message_id, session_id: m.session_id, outcome: "succeeded" })),
            received: ms.map((m) => ({ ...summarize(m), sequence_number: String(m.sequence_number) })),
            apply: (i) => removes && h.remove(f.entity, [ms[i].sequence_number]),
          }),
        });
      }
      case "op_settle_deferred": {
        const entity = args.entity as string;
        const sequences = args.sequences as string[];
        const how = args.how as Settlement;
        if (!sequences?.length) throw new EngineError("invalid", "Select the deferred messages to settle.");
        const sel = select({ entity, view: "deferred", selection: { type: "sequences", sequences } });
        return start(args.profileId, {
          kind: "settle-deferred", title: `Settle ${thousands(sequences.length)} deferred in ${entity}`, entity,
          asks: [{ verb: how.type, target: entity, summary: `${how.type === "complete" ? "Complete" : how.type === "abandon" ? "Abandon" : how.type === "defer" ? "Defer" : "Dead-letter"} ${thousands(sequences.length)} deferred ${sequences.length === 1 ? "message" : "messages"} in ${entity} on ${name(args.profileId)}`, count: sequences.length, reversible: how.type === "abandon" || how.type === "defer" }],
          work: () => ({
            items: [...sel.found.map(sel.items), ...sel.missing.map(sel.items)],
            apply: (i) => (how.type === "complete" || how.type === "dead-letter") && sel.found[i] && h.remove(entity, [sel.found[i].sequence_number]),
          }),
        });
      }
      case "op_delete": {
        const f = args.form as SelectForm;
        const sel = select(f);
        const words = f.selection.type === "sequences" ? thousands(f.selection.sequences.length) : "matching";
        const matched = sel.notes.length ? `; ${sel.notes[0]}; run again for the rest` : "";
        return start(args.profileId, {
          kind: "delete", title: `Delete ${words} from ${at(f.entity, f.view)}`, entity: f.entity,
          asks: [{ verb: "delete", target: f.entity, summary: `Delete ${messages(sel.count)} from ${at(f.entity, f.view)} on ${name(args.profileId)}${matched}`, count: sel.count, reversible: false }],
          work: () => ({
            items: [...sel.found.map(sel.items), ...sel.missing.map(sel.items)],
            notes: sel.notes,
            apply: (i) => sel.found[i] && h.remove(f.entity, [sel.found[i].sequence_number]),
          }),
        });
      }
      case "op_move": {
        const f = args.form as SelectForm;
        const target = String(args.target ?? "").trim();
        if (!target) throw new EngineError("invalid", "Choose where to move the messages.");
        if (target === f.entity && f.view !== "dead-letter" && f.view !== "transfer-dead-letter") throw new EngineError("invalid", "Choose another entity to move the messages to.");
        const sel = select(f);
        const words = f.selection.type === "sequences" ? thousands(f.selection.sequences.length) : "matching";
        const summary = `Move ${messages(sel.count)} from ${at(f.entity, f.view)} to ${target} on ${name(args.profileId)}`;
        return start(args.profileId, {
          kind: "move", title: `Move ${words} from ${at(f.entity, f.view)} to ${target}`, entity: f.entity, target,
          asks: [
            { verb: "move", target: f.entity, summary, count: sel.count, reversible: false },
            { verb: "move", target, summary, count: sel.count, reversible: false },
          ],
          work: () => ({
            items: [...sel.found.map((m) => ({ ...sel.items(m), new_sequence_number: String(nextNewSeq++) })), ...sel.missing.map(sel.items)],
            notes: sel.notes,
            apply: (i) => sel.found[i] && h.remove(f.entity, [sel.found[i].sequence_number]),
          }),
        });
      }
      case "op_resubmit": {
        const f = args.form as ResubmitForm;
        const sel = select(f.source);
        const target = f.target?.trim() || f.source.entity.split("/subscriptions/")[0];
        const words = f.source.selection.type === "sequences" ? thousands(f.source.selection.sequences.length) : "matching";
        const summary = `Resubmit ${messages(sel.count)} from ${at(f.source.entity, f.source.view)} to ${target}${f.remove_original ? " and remove the originals" : ""} on ${name(args.profileId)}`;
        return start(args.profileId, {
          kind: "resubmit", title: `Resubmit ${words} from ${at(f.source.entity, f.source.view)} to ${target}`, entity: f.source.entity, target,
          asks: [
            { verb: "resubmit", target: f.source.entity, summary, count: sel.count, reversible: !f.remove_original },
            ...(target !== f.source.entity ? [{ verb: "resubmit", target, summary, count: sel.count, reversible: !f.remove_original }] : []),
          ],
          work: () => ({
            items: [...sel.found.map((m) => ({ ...sel.items(m), new_sequence_number: String(nextNewSeq++) })), ...sel.missing.map(sel.items)],
            notes: sel.notes,
            apply: (i) => f.remove_original && sel.found[i] && h.remove(f.source.entity, [sel.found[i].sequence_number]),
          }),
        });
      }
      case "purge_plan": {
        h.connectedTo(args.profileId);
        const entities = args.entities as string[];
        const scope = args.scope as PurgeScope;
        if (!entities?.length) throw new EngineError("invalid", "Choose what to purge.");
        const views: PurgePlanView[] = entities.map((entity) => {
          const c = h.countsOf(entity);
          const subs: SubQueue[] = scope === "all" ? ["active", "dead-letter", "transfer-dead-letter"] : [scope];
          const n = (q: SubQueue) => (!c ? null : q === "active" ? c.active : q === "dead-letter" ? c.dead_letter : c.transfer_dead_letter);
          const lines = subs.map((q) => ({ sub_queue: q, count: n(q) === null ? "an unknown number of messages" : messages(n(q)!) }));
          const total = c ? subs.reduce((t, q) => t + (n(q) ?? 0), 0) : null;
          const deferred = scope === "all" ? (c ? `${thousands(h.messagesOf(entity).filter((m) => m.state === "deferred").length)} deferred messages` : "an unknown number of deferred messages") : null;
          return { entity, scope, lines, deferred, total: total === null ? "an unknown number of messages" : messages(total), unknown: total === null, requires_session: c ? false : null };
        });
        const planId = next("plan");
        plans.set(planId, { profileId: args.profileId as string, plans: views, at: Date.now() });
        return { plan_id: planId, plans: views } satisfies PurgePlans;
      }
      case "op_purge": {
        const planned = plans.get(args.planId as string);
        plans.delete(args.planId as string);
        if (!planned || planned.profileId !== args.profileId || Date.now() - planned.at > 600_000)
          throw new EngineError("not_found", "These counts are no longer current. Count again before purging.");
        const unknown = planned.plans.find((p) => p.unknown);
        if (unknown) throw new EngineError("invalid", `The count of ${unknown.entity} is unknown, so it can't be purged safely.`);
        const on = name(args.profileId);
        const entity = planned.plans.length === 1 ? planned.plans[0].entity : `${planned.plans.length} entities`;
        return start(args.profileId, {
          kind: "purge", title: `Purge ${entity}`, entity,
          asks: planned.plans.map((p) => ({
            verb: "purge", target: p.entity, count: null, reversible: false,
            summary: `Purge ${p.total} from ${p.entity}${p.scope === "all" ? "" : SUB_SUFFIX[p.scope as SubQueue]} on ${on}${p.scope === "all" ? ". Scheduled messages are kept; cancel them with `scheduled cancel --all`" : ""}`,
          })),
          work: () => {
            const ms = planned.plans.flatMap((p) => {
              const subs = p.scope === "all" ? ["main", "dlq", "tdlq"] : [subOf(p.scope as SubQueue)];
              return h.messagesOf(p.entity).filter((m) => subs.includes(m.sub) && m.state !== "scheduled").slice(0, 50).map((m) => ({ m, entity: p.entity }));
            });
            return {
              items: ms.map(({ m, entity: e }) => item(e, { sequence_number: String(m.sequence_number), message_id: m.message_id, outcome: "succeeded" })),
              apply: (i) => h.remove(ms[i].entity, [ms[i].m.sequence_number]),
            };
          },
        });
      }
      case "dlq_summary": {
        h.connectedTo(args.profileId);
        const sub = subOf(args.subQueue as SubQueue);
        const ms = h.messagesOf(args.entity as string).filter((m) => m.sub === sub);
        const groups = new Map<string, { reason: string | null; description: string | null; count: number; first: number; last: number }>();
        for (const m of ms) {
          const k = `${m.dead_letter_reason}|${m.dead_letter_description}`;
          const g = groups.get(k) ?? { reason: m.dead_letter_reason, description: m.dead_letter_description, count: 0, first: m.sequence_number, last: m.sequence_number };
          g.count++;
          g.first = Math.min(g.first, m.sequence_number);
          g.last = Math.max(g.last, m.sequence_number);
          groups.set(k, g);
        }
        return {
          groups: [...groups.values()]
            .sort((a, b) => b.count - a.count)
            .map((g) => ({ reason: g.reason, description: g.description, count: g.count, first_sequence: String(g.first), last_sequence: String(g.last) })),
          scanned: ms.length,
          truncated: false,
        } satisfies DlqSummaryView;
      }
      case "save_session_state":
        h.connectedTo(args.profileId);
        return `${args.session}-state.bin`;
      case "session_state_get": {
        h.connectedTo(args.profileId);
        const key = `${args.entity}|${args.session}`;
        let bytes = states.get(key);
        // RMA-1001 starts with a small JSON state.
        if (!bytes && args.session === "RMA-1001" && !states.has(`${key}|cleared`)) bytes = new TextEncoder().encode('{"step":"inspect","owner":"returns-team"}');
        if (!bytes) return null;
        let text: string | null = null;
        try {
          text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
        } catch {
          text = null;
        }
        return { base64: btoa(String.fromCharCode(...bytes)), text, len: bytes.length } satisfies SessionStateView;
      }
      case "op_session_state_set": {
        const entity = args.entity as string;
        const session = args.session as string;
        const value = args.value as string | null;
        let bytes: Uint8Array | null = null;
        if (value !== null) {
          try {
            bytes = Uint8Array.from(atob(value.trim()), (c) => c.charCodeAt(0));
          } catch {
            throw new EngineError("invalid", "The state is not valid base64.");
          }
          if (bytes.length > 256 * 1024 && !args.allowLarge)
            throw new EngineError("invalid", "The state is larger than 256 KiB. Turn on Allow large state if the namespace is Premium.");
        }
        const replace = bytes !== null;
        const key = `${entity}|${session}`;
        return start(args.profileId, {
          kind: "session-state", title: `${replace ? "Set" : "Clear"} the state of session ${session} in ${entity}`, entity,
          asks: [{ verb: replace ? "set-session-state" : "clear-session-state", target: entity, summary: `${replace ? "Replace" : "Clear"} the state of session ${session} in ${entity} on ${name(args.profileId)}`, count: null, reversible: false }],
          work: () => {
            if (bytes) states.set(key, bytes);
            else {
              states.delete(key);
              states.set(`${key}|cleared`, new Uint8Array());
            }
            return { items: [] };
          },
        });
      }
      case "resolve_kind":
        h.connectedTo(args.profileId);
        // `orders` is a topic the connection string names (`EntityPath`): listings show it as a queue.
        if (args.entity === "orders") return "topic";
        return h.kindOf(args.entity as string);
      case "pick_open_paths": {
        const purpose = args.purpose as OpenPurpose;
        const names = purpose === "body" ? ["order-1001.json", "order-1002.json"] : purpose === "template" ? ["order-template.json"] : ["orders-dead-letter-20261016-0941.json"];
        const pick = next("open");
        picks.set(pick, names);
        return { pick, names } satisfies OpenedFiles;
      }
      case "job_report": {
        const r = reports.get(args.id as string);
        if (!r) return null;
        const offset = Number(args.offset ?? 0);
        const limit = Math.min(Number(args.limit ?? 1000), 1000);
        const wanted = args.onlyProblems ? r.items.filter((i) => i.outcome !== "succeeded") : r.items;
        return { summary: r.summary, offset, total: wanted.length, items: wanted.slice(offset, offset + limit) } satisfies ReportPage;
      }
      case "pick_report_path":
      case "pick_activity_export_path": {
        const ext = args.format === "ndjson" ? "ndjson" : "json";
        if (cmd === "pick_report_path" && !reports.has(args.id as string)) throw new EngineError("not_found", "That job has no report.");
        const file = cmd === "pick_report_path" ? `${reports.get(args.id as string)!.summary.entity.replace(/[/$]/g, "-")}-${reports.get(args.id as string)!.summary.kind}-report.${ext}` : `dispatched-activity.${ext}`;
        const pick = next("save");
        const dir = "/Users/ops/Downloads";
        savePicks.set(pick, `${dir}/${file}`);
        return { pick, name: file, dir } satisfies PickedPath;
      }
      case "job_report_save": {
        const path = savePicks.get(args.pick as string);
        if (!path) throw new EngineError("invalid", "Choose where to save the report.");
        if (!reports.has(args.id as string)) throw new EngineError("not_found", "That job has no report.");
        savePicks.delete(args.pick as string);
        return path;
      }
      case "activity_list": {
        const q = (args.query ?? {}) as ActivityQuery;
        return log
          .filter((r) => (!q.origin || r.origin === q.origin) && (!q.profile || r.profile === q.profile) && (!q.outcome || r.outcome === q.outcome))
          .slice()
          .reverse()
          .slice(0, q.limit ?? 500);
      }
      case "activity_since": {
        const after = args.afterId as number | null;
        if (after === null || after === undefined) return log.slice(-1);
        return log.filter((r) => r.id > after).slice(0, 1000);
      }
      case "activity_export": {
        const path = savePicks.get(args.pick as string);
        if (!path) throw new EngineError("invalid", "Choose where to save the export.");
        savePicks.delete(args.pick as string);
        const q = (args.query ?? {}) as ActivityQuery;
        return log.filter((r) => (!q.origin || r.origin === q.origin) && (!q.profile || r.profile === q.profile) && (!q.outcome || r.outcome === q.outcome)).length;
      }
      default:
        return UNHANDLED;
    }
  }

  return {
    handle,
    /** As `Jobs::cancel` for an operate job: a waiting confirmation is answered "no". */
    cancel(id: string): boolean {
      const r = running.get(id);
      if (!r) return false;
      r.cancelled = true;
      r.deny?.();
      return true;
    },
    /** The connection closes: every operate job stops. */
    closeAll() {
      for (const r of running.values()) {
        r.stopped = true;
        r.deny?.();
      }
    },
    /** A change made elsewhere (the CLI or an agent), written to the shared log. */
    external(r: Omit<ActivityRecord, "id" | "at">) {
      return record(r);
    },
  };
}

export const UNHANDLED = Symbol("unhandled");
