import { useState } from "react";
import { EngineError, commands, type EntityRefForm, type ImportPreview } from "@/engine";
import { Button, EntityGlyph, Problem, Sheet, formatNumber } from "@/ui";
import { useShell } from "../context";
import { Check } from "../operate/controls";
import { OperationSheet } from "../operate/OperationSheet";
import { useOperation } from "../operate/useOperation";

/** The `serviceBusNamespace` of an export: the first label of the namespace host. */
const namespaceOf = (host: string) => host.split(".")[0] || "namespace";

/**
 * Export queues, topics, subscriptions and their rules as a ServiceBusExplorer `Entities` file, which the incumbent and
 * Dispatched import. A read: nothing is confirmed. Keys are left out unless asked for.
 */
export function ExportEntitiesSheet({ entities }: { entities: EntityRefForm[] }) {
  const { conn, closeSheet } = useShell();
  const [includeKeys, setIncludeKeys] = useState(false);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const run = async () => {
    if (!conn.profileId) return;
    setFailure(null);
    setBusy(true);
    try {
      const picked = await commands.pickEntitiesExportPath(namespaceOf(conn.view?.host ?? ""));
      if (!picked) return;
      const r = await commands.entitiesExport(conn.profileId, entities, includeKeys, picked.pick);
      setDone(`Exported ${formatNumber(r.count)} ${r.count === 1 ? "entity" : "entities"} to ${r.file_name}${includeKeys ? ", with keys" : ""}.`);
    } catch (e) {
      setFailure(e instanceof EngineError || e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  const footer = done ? (
    <Button variant="primary" onClick={closeSheet}>
      Close
    </Button>
  ) : (
    <>
      <Button onClick={closeSheet}>Cancel</Button>
      <Button variant="primary" disabled={!conn.profileId || busy} aria-busy={busy} onClick={() => void run()}>
        Export…
      </Button>
    </>
  );
  return (
    <Sheet open onOpenChange={(o) => !o && closeSheet()} title="Export entities" description="A ServiceBusExplorer file: settings, subscriptions, rules and policies. Messages are not exported." footer={footer} dismissible={!busy}>
      <div className="flex flex-col gap-4">
        {failure && <Problem role="alert" title="Couldn't export the entities" body="Nothing was written. Check the connection, then try again." detail={failure} />}
        {done ? (
          <p role="status" className="m-0 text-12 text-fg">{done}</p>
        ) : (
          <>
            {entities.length === 0 ? (
              <p className="m-0 text-12 text-fg">Every queue and topic of the namespace, with their subscriptions and rules.</p>
            ) : (
              <ul aria-label="Entities to export" className="m-0 flex list-none flex-col gap-1 p-0">
                {entities.map((e) => (
                  <li key={e.entity} className="flex items-center gap-2 text-12">
                    <EntityGlyph kind={e.kind} className="size-3.5 shrink-0 text-icon" />
                    <span className="font-mono text-11">{e.entity}</span>
                  </li>
                ))}
              </ul>
            )}
            <Check label="Include keys" hint="Writes the policies' keys, so an import keeps them." checked={includeKeys} onChange={setIncludeKeys} />
            {includeKeys && <p className="m-0 text-12 text-fg">The file will hold the keys of every policy. Anyone with the file can send and receive. It is saved readable only by you.</p>}
          </>
        )}
      </div>
    </Sheet>
  );
}

/** Import a ServiceBusExplorer `Entities` file: read and checked first, then created in one confirmed job. */
export function ImportEntitiesSheet() {
  const { conn, closeSheet } = useShell();
  const op = useOperation();
  const [preview, setPreview] = useState<{ file: string; p: ImportPreview } | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [overwrite, setOverwrite] = useState(false);
  const choose = async () => {
    if (!conn.profileId) return;
    setProblem(null);
    try {
      const opened = await commands.pickOpenPaths("entities");
      if (!opened) return;
      const p = await commands.entitiesImportPreview(conn.profileId, opened.pick);
      setPreview({ file: opened.names[0] ?? "", p });
    } catch (e) {
      setPreview(null);
      setProblem(e instanceof Error ? e.message : String(e));
    }
  };
  const n = preview?.p.entities.length ?? 0;
  return (
    <OperationSheet
      title="Import entities"
      description="Creates the queues, topics, subscriptions and rules of a ServiceBusExplorer export. Messages are not part of it."
      op={op}
      right="Manage"
      action={n ? `Import ${formatNumber(n)} ${n === 1 ? "entity" : "entities"}` : "Import"}
      canSubmit={!!conn.profileId && !!preview}
      onSubmit={() => preview && void op.start(() => commands.opEntitiesImport(conn.profileId!, preview.p.preview_id, overwrite))}
      onClose={closeSheet}
    >
      <div className="flex items-center gap-2">
        <Button onClick={() => void choose()}>Choose file…</Button>
        {preview && <span className="truncate font-mono text-11 text-muted">{preview.file}</span>}
      </div>
      {problem && <Problem role="alert" title="Couldn't read the file" body="Nothing was imported. The file was refused as a whole; the detail says why." detail={problem} />}
      {preview && (
        <>
          <table aria-label="Entities in the file" className="w-full border-collapse text-12">
            <thead>
              <tr className="h-row border-b border-line text-left text-11 text-muted">
                <th className="font-normal">Entity</th>
                <th className="w-14 text-right font-normal">Rules</th>
                <th className="w-16 text-right font-normal">Policies</th>
              </tr>
            </thead>
            <tbody>
              {preview.p.entities.map((e) => (
                <tr key={e.entity} className="h-row border-b border-line">
                  <td className="flex items-center gap-2 py-1">
                    <EntityGlyph kind={e.kind} className="size-3.5 shrink-0 text-icon" />
                    <span className="truncate font-mono text-11">{e.entity}</span>
                  </td>
                  <td className="text-right tabular-nums">{e.kind === "subscription" ? e.rules : ""}</td>
                  <td className="text-right tabular-nums">{e.kind === "subscription" ? "" : `${e.policies}${e.keys ? " with keys" : ""}`}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <Check
            label="Replace existing entities"
            hint="Existing entities get the file's settings; policies without keys in the file keep their keys. Otherwise existing ones are skipped."
            checked={overwrite}
            onChange={setOverwrite}
          />
        </>
      )}
    </OperationSheet>
  );
}
