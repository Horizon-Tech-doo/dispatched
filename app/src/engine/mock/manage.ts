import { EngineError } from "../ipc";
import type {
  EntityKindName, EntityRefForm, EntityStatus, ImportPreview, JobStarted, ListenForm, MessageDetail, PolicyChange, PolicyView, ReceivedView, RuleFilter,
  RuleForm, RuleInfo, SettingName, SettingsChange, SettingsView, StoredProfile,
} from "../types";
import { summarize } from "./messages";
import { UNHANDLED, type Ask, type createOperateMock } from "./operate";

/** The namespace as the manage mock changes it. Entity ids are tab ids (`topic/subscriptions/name`). */
export interface NamespaceHost {
  connectedTo(profileId: unknown): StoredProfile;
  kindOf(entity: string): EntityKindName | null;
  /** Adds a queue, topic or subscription to the listing. */
  add(kind: EntityKindName, entity: string): void;
  /** Takes an entity (and a topic's subscriptions) out of the listing. */
  remove(entity: string): void;
  rules(entity: string): RuleInfo[];
  setRules(entity: string, rules: RuleInfo[]): void;
  /** Takes the next file the open dialog "returns" for an import (its XML text). */
  openedFile(pick: string): string | null;
}

type Operate = ReturnType<typeof createOperateMock>;

/** Which settings each kind has, in the core's order. */
const FIELDS: Record<EntityKindName, SettingName[]> = {
  queue: [
    "lock_duration", "max_size_in_megabytes", "requires_duplicate_detection", "requires_session", "default_message_time_to_live",
    "dead_lettering_on_message_expiration", "duplicate_detection_history_time_window", "max_delivery_count", "enable_batched_operations", "status",
    "forward_to", "forward_dead_lettered_messages_to", "auto_delete_on_idle", "enable_partitioning", "enable_express", "max_message_size_in_kilobytes",
    "user_metadata",
  ],
  topic: [
    "max_size_in_megabytes", "requires_duplicate_detection", "default_message_time_to_live", "duplicate_detection_history_time_window",
    "enable_batched_operations", "status", "auto_delete_on_idle", "enable_partitioning", "enable_express", "support_ordering",
    "max_message_size_in_kilobytes", "user_metadata",
  ],
  subscription: [
    "lock_duration", "requires_session", "default_message_time_to_live", "dead_lettering_on_message_expiration",
    "dead_lettering_on_filter_evaluation_exceptions", "max_delivery_count", "enable_batched_operations", "status", "forward_to",
    "forward_dead_lettered_messages_to", "auto_delete_on_idle", "user_metadata",
  ],
};
const FIXED: SettingName[] = ["requires_duplicate_detection", "requires_session", "enable_partitioning"];
const DEFAULTS: SettingsChange = {
  lock_duration: "1m", max_size_in_megabytes: 1024, requires_duplicate_detection: false, requires_session: false, default_message_time_to_live: "max",
  dead_lettering_on_message_expiration: false, dead_lettering_on_filter_evaluation_exceptions: true, max_delivery_count: 10, enable_batched_operations: true,
  status: "Active", auto_delete_on_idle: "max", enable_partitioning: false, enable_express: false, support_ordering: true,
};
const DURATION = /^(max|never|(\d+(\.\d+)?[dhms])+|P.*)$/i;
const isDuration = (f: SettingName) => ["lock_duration", "default_message_time_to_live", "duplicate_detection_history_time_window", "auto_delete_on_idle"].includes(f);

/** As the core's checks: values that do not parse, settings the kind lacks, ones fixed at creation. */
export function settingsIssues(kind: EntityKindName, form: SettingsChange, create: boolean): { field: string; message: string }[] {
  const out: { field: string; message: string }[] = [];
  for (const [k, v] of Object.entries(form) as [SettingName, unknown][]) {
    if (!FIELDS.queue.includes(k) && !FIELDS.topic.includes(k) && !FIELDS.subscription.includes(k)) out.push({ field: k, message: `unknown field \`${k}\`` });
    else if (isDuration(k) && (typeof v !== "string" || !DURATION.test(v.trim()) || /^0+[dhms]$/.test(v.trim())))
      out.push({ field: k, message: `'${String(v)}' is not a duration; use 90s, 15m, 2h, 7d, 1h30m, PT1H30M or [d.]hh:mm:ss` });
    else if (!FIELDS[kind].includes(k)) out.push({ field: k, message: `A ${kind} has no such setting.` });
    else if (!create && FIXED.includes(k)) out.push({ field: k, message: "Set when the entity is created; it cannot change." });
  }
  return out.sort((a, b) => a.field.localeCompare(b.field));
}

/** As core `validate_filter` for the editor's forms. */
export function ruleIssues(rule: RuleForm): { field: string; message: string }[] {
  const out: { field: string; message: string }[] = [];
  if (!rule.name.trim()) out.push({ field: "name", message: "A rule needs a name." });
  const f = rule.filter;
  if (f.kind === "sql" && !f.expression.trim()) out.push({ field: "filter", message: "invalid input: a SQL filter needs an expression" });
  if (f.kind === "correlation") {
    const set = ["correlation_id", "message_id", "to", "reply_to", "subject", "session_id", "reply_to_session_id", "content_type"].some(
      (k) => (f[k as keyof typeof f] as string | null | undefined)?.trim(),
    );
    if (f.properties.some((p) => !p.name.trim()))
      out.push({ field: "filter", message: "invalid input: a correlation filter property needs a name" });
    else if (!set && !f.properties.length)
      out.push({
        field: "filter",
        message: "invalid input: this correlation filter sets no field and no property, so it matches no message; use a SQL filter 1=1 or the default rule to match every message",
      });
  }
  return out;
}

const plain = (f: RuleForm["filter"]): RuleFilter =>
  f.kind === "correlation"
    ? {
        kind: "correlation", correlation_id: f.correlation_id || null, message_id: f.message_id || null, to: f.to || null, reply_to: f.reply_to || null,
        subject: f.subject || null, session_id: f.session_id || null, reply_to_session_id: f.reply_to_session_id || null, content_type: f.content_type || null,
        properties: f.properties.map((p) => ({ name: p.name, value: p.value })),
      }
    : f;

/** The core's path of an entity: `topic/Subscriptions/name`. */
const pathOf = (id: string) => id.replace("/subscriptions/", "/Subscriptions/");

export function createManageMock(h: NamespaceHost, op: Operate, tick: () => number) {
  const settings = new Map<string, SettingsChange>();
  const policies = new Map<string, PolicyView[]>([
    ["invoice-generate", [{ key_name: "billing-app", rights: ["listen", "send"] }, { key_name: "ops", rights: ["listen", "send", "manage"] }]],
  ]);
  const previews = new Map<string, { profileId: string; rows: ImportPreview["entities"] }>();
  /** Messages a removing listener took, by job id. */
  const held = new Map<string, Map<string, MessageDetail>>();
  let seq = 0;
  const name = (p: unknown) => h.connectedTo(p).name;
  const must = (entity: string) => {
    const k = h.kindOf(entity);
    if (!k) throw new EngineError("not_found", `not found: ${pathOf(entity)}`);
    return k;
  };
  const ask = (verb: string, target: string, summary: string, count: number | null = 1): Ask => ({ verb, target: pathOf(target), summary, count, reversible: false });
  const one = (profileId: unknown, kind: Parameters<Operate["start"]>[1]["kind"], title: string, entity: string, asks: Ask[], apply: () => void): JobStarted =>
    op.start(profileId, {
      kind, title, entity: pathOf(entity), asks,
      work: () => ({ items: [op.item(pathOf(entity), { outcome: "succeeded" })], apply }),
    });
  const viewOf = (kind: EntityKindName, entity: string): SettingsView => {
    const s = { ...DEFAULTS, ...settings.get(entity) };
    return {
      kind, entity,
      settings: Object.fromEntries(FIELDS[kind].map((f) => [f, s[f] ?? null])),
      fixed: FIELDS[kind].filter((f) => FIXED.includes(f)),
    };
  };

  function handle(cmd: string, args: Record<string, unknown>): unknown {
    switch (cmd) {
      case "check_settings":
        return settingsIssues(args.kind as EntityKindName, args.form as SettingsChange, args.create as boolean);
      case "check_rule":
        return ruleIssues(args.rule as RuleForm);
      case "entity_settings": {
        h.connectedTo(args.profileId);
        const entity = args.entity as string;
        must(entity);
        return viewOf(args.kind as EntityKindName, entity);
      }
      case "entity_policies": {
        h.connectedTo(args.profileId);
        if (args.kind === "subscription") throw new EngineError("invalid", "Subscriptions have no shared access policies.");
        must(args.entity as string);
        return structuredClone(policies.get(args.entity as string) ?? []);
      }
      case "copy_policy_secret": {
        h.connectedTo(args.profileId);
        const p = (policies.get(args.entity as string) ?? []).find((x) => x.key_name === args.policy);
        if (!p) throw new EngineError("not_found", `${args.entity} has no policy ${args.policy}.`);
        return null;
      }
      case "op_entity_create": {
        const kind = args.kind as EntityKindName;
        const entity = (args.entity as string).trim();
        const issues = settingsIssues(kind, args.settings as SettingsChange, true);
        if (issues.length) throw new EngineError("invalid", `${issues[0].field}: ${issues[0].message}`);
        if (args.rule) {
          const r = ruleIssues(args.rule as RuleForm);
          if (r.length) throw new EngineError("invalid", r[0].message);
        }
        if (h.kindOf(entity)) throw new EngineError("conflict", `${pathOf(entity)} already exists`);
        const rule = args.rule as RuleForm | null;
        return one(args.profileId, "create-entities", `Create ${kind} ${pathOf(entity)}`, entity, [ask("create-entity", entity, `Create ${kind} ${pathOf(entity)} on ${name(args.profileId)}`)], () => {
          h.add(kind, entity);
          settings.set(entity, { ...(args.settings as SettingsChange) });
          if (kind === "subscription")
            h.setRules(entity, [rule ? { name: rule.name, filter: plain(rule.filter), action: rule.action, created_at: null } : { name: "$Default", filter: { kind: "true" }, action: null, created_at: null }]);
        });
      }
      case "op_entity_update": {
        const kind = args.kind as EntityKindName;
        const entity = args.entity as string;
        const changes = args.changes as SettingsChange;
        if (!Object.keys(changes).length) throw new EngineError("invalid", "Nothing to change.");
        const issues = settingsIssues(kind, changes, false);
        if (issues.length) throw new EngineError("invalid", `${issues[0].field}: ${issues[0].message}`);
        must(entity);
        return one(args.profileId, "update-entities", `Update ${kind} ${pathOf(entity)}`, entity, [ask("update-entity", entity, `Update ${kind} ${pathOf(entity)} on ${name(args.profileId)}`)], () =>
          settings.set(entity, { ...settings.get(entity), ...changes }),
        );
      }
      case "op_entity_status": {
        const t = args.target as EntityRefForm;
        const status = args.status as EntityStatus;
        must(t.entity);
        return one(args.profileId, "update-entities", `Set ${t.kind} ${pathOf(t.entity)} to ${status}`, t.entity, [ask("set-status", t.entity, `Set ${t.kind} ${pathOf(t.entity)} to ${status} on ${name(args.profileId)}`)], () =>
          settings.set(t.entity, { ...settings.get(t.entity), status }),
        );
      }
      case "op_entity_copy": {
        const t = args.target as EntityRefForm;
        const rename = args.rename as boolean;
        const newName = (args.newName as string).trim();
        if (!newName || newName.includes("/")) throw new EngineError("invalid", "The new name must be a plain name, without '/'.");
        must(t.entity);
        if (rename && t.kind === "subscription") throw new EngineError("invalid", "rename works on queues and topics; duplicate the subscription, then delete it");
        if (!rename && t.kind === "topic") throw new EngineError("invalid", "duplicate works on queues and subscriptions; create the topic and its subscriptions instead");
        const to = t.kind === "subscription" ? `${t.entity.split("/subscriptions/")[0]}/subscriptions/${newName}` : newName;
        if (h.kindOf(to)) throw new EngineError("conflict", `${pathOf(to)} already exists`);
        const at = name(args.profileId);
        const asks = rename
          ? [ask("rename", t.entity, `Rename ${t.kind} ${pathOf(t.entity)} to ${pathOf(to)} (create ${pathOf(to)}, then delete ${pathOf(t.entity)}) on ${at}`), ask("rename", to, `Rename ${t.kind} ${pathOf(t.entity)} to ${pathOf(to)} on ${at}`)]
          : [ask("duplicate", to, `Duplicate ${t.kind} ${pathOf(t.entity)} as ${newName} on ${at} (settings and policies, with new keys; messages are not copied)`)];
        return one(args.profileId, rename ? "rename" : "duplicate", `${rename ? "Rename" : "Duplicate"} ${t.kind} ${pathOf(t.entity)} as ${newName}`, t.entity, asks, () => {
          h.add(t.kind, to);
          settings.set(to, { ...settings.get(t.entity) });
          if (t.kind === "subscription") h.setRules(to, h.rules(t.entity));
          if (rename) h.remove(t.entity);
        });
      }
      case "op_entity_delete": {
        const targets = args.entities as EntityRefForm[];
        if (!targets.length) throw new EngineError("invalid", "Choose what to delete.");
        targets.forEach((t) => must(t.entity));
        const at = name(args.profileId);
        const summary = `Delete ${targets.map((t) => `${t.kind} ${pathOf(t.entity)}`).join(", ")} and their messages on ${at}`;
        return op.start(args.profileId, {
          kind: "delete-entities",
          title: targets.length === 1 ? `Delete ${targets[0].kind} ${pathOf(targets[0].entity)}` : `Delete ${targets.length} entities`,
          entity: targets.length === 1 ? pathOf(targets[0].entity) : `${targets.length} entities`,
          asks: targets.map((t) => ask("delete-entity", t.entity, summary, null)),
          work: () => ({ items: targets.map((t) => op.item(pathOf(t.entity), { outcome: "succeeded" })), apply: (i) => h.remove(targets[i].entity) }),
        });
      }
      case "op_rule_save": {
        const entity = args.entity as string;
        const rule = args.rule as RuleForm;
        const update = args.update as boolean;
        const issues = ruleIssues(rule);
        if (issues.length) throw new EngineError("invalid", issues[0].message);
        must(entity);
        const rules = h.rules(entity);
        if (update !== rules.some((r) => r.name === rule.name))
          throw new EngineError(update ? "not_found" : "conflict", update ? `not found: rule ${rule.name}` : `rule ${rule.name} already exists`);
        const verb = update ? "update-rule" : "add-rule";
        const summary = update ? `Update rule ${rule.name} of ${pathOf(entity)} on ${name(args.profileId)}` : `Add rule ${rule.name} to ${pathOf(entity)} on ${name(args.profileId)}`;
        return one(args.profileId, "update-entities", update ? `Update rule ${rule.name} of ${pathOf(entity)}` : `Add rule ${rule.name} of ${pathOf(entity)}`, entity, [ask(verb, entity, summary)], () => {
          const info: RuleInfo = { name: rule.name, filter: plain(rule.filter), action: rule.action || null, created_at: null };
          h.setRules(entity, update ? h.rules(entity).map((r) => (r.name === rule.name ? info : r)) : [...h.rules(entity), info]);
        });
      }
      case "op_rule_remove": {
        const entity = args.entity as string;
        const rule = args.name as string;
        must(entity);
        return one(args.profileId, "update-entities", `Remove rule ${rule} from ${pathOf(entity)}`, entity, [ask("remove-rule", entity, `Remove rule ${rule} from ${pathOf(entity)} on ${name(args.profileId)}`)], () =>
          h.setRules(entity, h.rules(entity).filter((r) => r.name !== rule)),
        );
      }
      case "op_policy_change": {
        const t = args.target as EntityRefForm;
        const policy = args.name as string;
        const change = args.change as PolicyChange;
        if (t.kind === "subscription") throw new EngineError("invalid", "subscriptions have no shared access policies");
        must(t.entity);
        const list = policies.get(t.entity) ?? [];
        if (change.type === "add") {
          if (!policy.trim()) throw new EngineError("invalid", "a policy needs a name");
          if (list.some((p) => p.key_name === policy)) throw new EngineError("invalid", `policy ${policy} already exists`);
          if (list.length >= 12) throw new EngineError("invalid", "an entity holds at most 12 policies");
          if (change.rights.includes("manage") && !(change.rights.includes("listen") && change.rights.includes("send")))
            throw new EngineError("invalid", "Manage needs Listen and Send as well");
        }
        const at = name(args.profileId);
        const [verb, summary] =
          change.type === "add"
            ? ["add-policy", `Add policy ${policy} (${change.rights.join(", ")}) to ${t.entity} on ${at}`]
            : change.type === "remove"
              ? ["remove-policy", `Remove policy ${policy} from ${t.entity} on ${at}; clients using its keys stop working`]
              : ["regenerate-key", `Regenerate the ${change.slot} key of policy ${policy} on ${t.entity} on ${at}; clients using it stop working`];
        return one(args.profileId, "update-entities", summary.split(" on ")[0], t.entity, [ask(verb, t.entity, summary)], () => {
          if (change.type === "add") policies.set(t.entity, [...list, { key_name: policy, rights: change.rights }]);
          if (change.type === "remove") policies.set(t.entity, list.filter((p) => p.key_name !== policy));
        });
      }
      case "pick_entities_export_path":
        return { pick: `pick-entities-${++seq}`, name: `${args.namespace}-entities.xml`, dir: "/Users/me/Downloads" };
      case "entities_export": {
        h.connectedTo(args.profileId);
        const n = (args.entities as EntityRefForm[]).length || 7;
        return { count: n, file_name: "logistics-entities.xml" };
      }
      case "entities_import_preview": {
        h.connectedTo(args.profileId);
        const text = h.openedFile(args.pick as string);
        if (text === null) throw new EngineError("invalid", "Choose the file again.");
        if (text.includes("broken")) throw new EngineError("invalid", "invalid input: queue orders, LockDuration: '1 minute' is not a TimeSpan");
        const rows: ImportPreview["entities"] = [
          { kind: "queue", entity: "orders-import", rules: 0, policies: 1, keys: false },
          { kind: "topic", entity: "events-import", rules: 0, policies: 0, keys: false },
          { kind: "subscription", entity: "events-import/Subscriptions/all", rules: 2, policies: 0, keys: false },
        ];
        const id = `import-${++seq}`;
        previews.set(id, { profileId: args.profileId as string, rows });
        return { preview_id: id, entities: rows } satisfies ImportPreview;
      }
      case "op_entities_import": {
        const p = previews.get(args.previewId as string);
        previews.delete(args.previewId as string);
        if (!p || p.profileId !== args.profileId) throw new EngineError("not_found", "Choose the file again: it was read for another import.");
        const at = name(args.profileId);
        return op.start(args.profileId, {
          kind: "import-entities", title: `Import ${p.rows.length} entities`, entity: `${p.rows.length} entities`,
          asks: [ask("import-entities", p.rows[0].entity, `Create ${p.rows.map((r) => r.entity).join(", ")} on ${at}${args.overwrite ? "; existing ones are replaced" : ""}`, p.rows.length)],
          work: () => ({
            items: p.rows.map((r) => op.item(r.entity, h.kindOf(r.entity.replace("/Subscriptions/", "/subscriptions/")) && !args.overwrite ? { outcome: "skipped", reason: `${r.entity} already exists; use --overwrite to replace it` } : { outcome: "succeeded" })),
            apply: (i) => h.add(p.rows[i].kind, p.rows[i].entity.replace("/Subscriptions/", "/subscriptions/")),
          }),
        });
      }
      case "op_listen": {
        const f = args.form as ListenForm;
        must(f.entity);
        if (f.concurrency < 1 || f.concurrency > 8) throw new EngineError("invalid", "use 1 to 8 receivers");
        if (f.mode === "complete" && f.prefetch > 0) throw new EngineError("invalid", "prefetch works only with receive and delete");
        const how = f.mode === "complete" ? "receive and complete" : "receive and delete";
        const asks = f.mode === "watch" ? [] : [ask(`listen-${f.mode}`, f.entity, `Remove every message that arrives on ${pathOf(f.entity)} until stopped, on ${name(args.profileId)} (${how})`, null)];
        const title = f.mode === "watch" ? `Watch ${f.entity}` : `Listen to ${f.entity} (${how})`;
        let jobId = "";
        const started = op.startStream(args.profileId, {
          title, entity: pathOf(f.entity), asks, every: tick() || 25,
          next: (n) => {
            const m = listenedMessage(n);
            if (f.mode !== "watch") held.get(jobId)?.set(m.view.sequence_number, m.detail);
            return m.view;
          },
        });
        jobId = started.id;
        if (f.mode !== "watch") held.set(jobId, new Map());
        return started;
      }
      case "job_message":
        return held.get(args.id as string)?.get(args.sequence as string) ?? null;
      default:
        return UNHANDLED;
    }
  }

  return { handle };
}

/** The n-th message a mock listener sees: its full detail and its received view. */
function listenedMessage(n: number): { detail: MessageDetail; view: ReceivedView } {
  const text = JSON.stringify({ event: "order-created", order: 7000 + n });
  const detail: MessageDetail = {
    sequence_number: 50_000 + n, state: "active", message_id: `msg-${7000 + n}`, correlation_id: null, session_id: null, subject: "order-created",
    content_type: "application/json", to: null, reply_to: null, reply_to_session_id: null, partition_key: null, expires_at: null,
    enqueued_time: new Date(Date.UTC(2026, 9, 20, 8, 0, 0) + n * 1000).toISOString(), scheduled_enqueue_time: null, locked_until: null,
    delivery_count: 1, time_to_live_ms: null, dead_letter_reason: null, dead_letter_description: null, dead_letter_source: null,
    application_properties: { region: { type: "string", value: "eu" } }, body: { kind: "text", value: text }, body_size: text.length, body_truncated: false,
  };
  const summary = summarize({ ...detail, sub: "main" });
  return { detail, view: { ...summary, sequence_number: String(summary.sequence_number) } };
}
