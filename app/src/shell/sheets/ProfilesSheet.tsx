import { useProfiles } from "@/engine";
import { Button, LabelSwatch, ProdBadge, Sheet } from "@/ui";
import { shortHost } from "../ConnectionSelector";
import { useShell } from "../context";

export function ProfilesSheet() {
  const { closeSheet, openSheet, conn } = useShell();
  const profiles = useProfiles().data ?? [];
  return (
    <Sheet
      open
      onOpenChange={(o) => !o && closeSheet()}
      title="Connections"
      footer={<Button variant="primary" onClick={() => openSheet({ kind: "profile", profileId: null })}>Add connection</Button>}
    >
      {profiles.length === 0 ? (
        <p className="m-0 text-12 text-muted">No connections yet. Add one to get started.</p>
      ) : (
        <ul className="m-0 flex list-none flex-col border-y border-line p-0">
          {profiles.map((p) => (
            <li key={p.id} className="flex h-9 items-center gap-2 px-1 text-12">
              <LabelSwatch color={p.color} />
              <span className="font-semibold">{p.name}</span>
              <span className="truncate text-muted">{shortHost(p.host)}</span>
              {p.production && <ProdBadge />}
              <span className="flex-1" />
              <Button size="sm" variant="quiet" aria-label={`Edit ${p.name}`} onClick={() => openSheet({ kind: "profile", profileId: p.id })}>Edit</Button>
              <Button
                size="sm"
                aria-label={`Connect to ${p.name}`}
                disabled={conn.profileId === p.id && conn.status === "connected"}
                onClick={() => {
                  closeSheet();
                  void conn.connect(p.id);
                }}
              >
                Connect
              </Button>
            </li>
          ))}
        </ul>
      )}
    </Sheet>
  );
}
