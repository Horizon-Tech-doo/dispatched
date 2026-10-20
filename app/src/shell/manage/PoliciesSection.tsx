import { useQuery } from "@tanstack/react-query";
import { MoreHorizontal } from "lucide-react";
import { commands, keys, type EntityKindName, type KeySlot, type PolicySecret, type PolicyView } from "@/engine";
import { Button, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger, IconButton, Problem, SkeletonRows } from "@/ui";
import { useShell } from "../context";
import { POLICY_LIMIT } from "./PolicySheet";

const RIGHT = { listen: "Listen", send: "Send", manage: "Manage" } as const;
export const rightsText = (p: PolicyView) => (["listen", "send", "manage"] as const).filter((r) => p.rights.includes(r)).map((r) => RIGHT[r]).join(", ");

const H2 = "m-0 mb-1.5 text-12 font-semibold text-muted";

/** An entity's own shared access policies: names and rights, and copying a key or connection string (Rust writes the
 *  clipboard, so a key is never shown or held here). */
export function PoliciesSection({ kind, entity }: { kind: EntityKindName; entity: string }) {
  const { conn, openSheet, say } = useShell();
  const q = useQuery({
    queryKey: keys.policies(conn.profileId, entity),
    queryFn: () => commands.entityPolicies(conn.profileId!, kind, entity),
    enabled: conn.profileId !== null,
  });
  const copy = async (p: PolicyView, slot: KeySlot, what: PolicySecret) => {
    try {
      await commands.copyPolicySecret(conn.profileId!, kind, entity, p.key_name, slot, what);
      say({ text: `Copied the ${slot} ${what === "key" ? "key" : "connection string"} of ${p.key_name}. It was never shown.` });
    } catch {
      say({ text: `Couldn't copy the ${what === "key" ? "key" : "connection string"} of ${p.key_name}`, error: true, details: true });
    }
  };
  const change = (p: PolicyView, c: { type: "remove" } | { type: "renew"; slot: KeySlot }) =>
    openSheet({ kind: "operation", op: "policy-change", entityKind: kind, entity, name: p.key_name, change: c });
  return (
    <section aria-label="Shared access policies">
      <h2 className={H2}>Shared access policies</h2>
      {q.isPending ? (
        <SkeletonRows rows={2} />
      ) : q.isError ? (
        <Problem title="Couldn't read the policies" body="Reading an entity's policies needs Manage." detail={q.error.message} />
      ) : q.data.length === 0 ? (
        <p className="m-0 text-12 text-muted">No policies of its own. Clients use the namespace's policies.</p>
      ) : (
        <table className="w-full border-collapse text-12">
          <thead className="sr-only">
            <tr>
              <th>Name</th>
              <th>Rights</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {q.data.map((p) => (
              <tr key={p.key_name} className="h-row border-b border-line">
                <td className="pr-3 font-mono text-11">{p.key_name}</td>
                <td className="pr-3 text-muted">{rightsText(p)}</td>
                <td className="w-6 text-right">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <IconButton label={`Actions for policy ${p.key_name}`}>
                        <MoreHorizontal aria-hidden="true" />
                      </IconButton>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem onSelect={() => void copy(p, "primary", "connection-string")}>Copy primary connection string</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => void copy(p, "secondary", "connection-string")}>Copy secondary connection string</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => void copy(p, "primary", "key")}>Copy primary key</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => void copy(p, "secondary", "key")}>Copy secondary key</DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem onSelect={() => change(p, { type: "renew", slot: "primary" })}>Regenerate primary key…</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => change(p, { type: "renew", slot: "secondary" })}>Regenerate secondary key…</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => change(p, { type: "remove" })}>Remove…</DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      <div className="mt-2 flex items-center gap-2">
        <Button disabled={!conn.profileId || (q.data?.length ?? 0) >= POLICY_LIMIT} onClick={() => openSheet({ kind: "operation", op: "policy-add", entityKind: kind, entity })}>
          Add policy…
        </Button>
        <span className="text-11 text-muted">Keys go straight to the clipboard; they are never shown.</span>
      </div>
    </section>
  );
}
