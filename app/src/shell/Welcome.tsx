import { useProfiles } from "@/engine";
import { Button, EmptyState, LabelSwatch, ProdBadge, SkeletonRows } from "@/ui";
import { shortHost } from "./ConnectionSelector";
import { describeConnectError } from "./connectError";
import { useShell } from "./context";

export function Welcome() {
  const { conn, openSheet } = useShell();
  const profiles = useProfiles();
  const failed = conn.status === "failed" ? profiles.data?.find((p) => p.id === conn.profileId) : undefined;

  if (profiles.isPending) {
    return (
      <div className="flex h-full w-full justify-center">
        <div className="w-[480px] max-w-full py-16" aria-busy="true">
          <SkeletonRows rows={4} />
        </div>
      </div>
    );
  }
  const edit = () => failed && openSheet({ kind: "profile", profileId: failed.id });
  const failure = failed ? describeConnectError(conn.error, failed) : null;

  return (
    <div className="flex h-full w-full justify-center overflow-y-auto">
      <div className="flex w-[480px] max-w-full flex-col gap-4 py-16">
        {failed && (
          <EmptyState
            tone="error"
            title={`Couldn't connect to ${failed.name}`}
            body={<span title={conn.error?.message}>{failure?.message}</span>}
            action={
              <div className="flex gap-2">
                {failure?.retryable ? (
                  <>
                    <Button onClick={conn.retry}>Try again</Button>
                    <Button variant="quiet" onClick={edit}>Edit connection</Button>
                  </>
                ) : (
                  <>
                    <Button onClick={edit}>Edit connection</Button>
                    <Button variant="quiet" onClick={conn.retry}>Try again</Button>
                  </>
                )}
              </div>
            }
          />
        )}
        {profiles.data && profiles.data.length === 0 ? (
          <EmptyState
            title="Add a connection"
            body="Use a namespace connection string, or sign in with Entra ID to use your Azure role assignments. Connections are saved on this computer; secrets go to the system keychain."
            action={
              <div className="flex gap-2">
                <Button variant="primary" onClick={() => openSheet({ kind: "profile", profileId: null, auth: "connection-string" })}>Add connection string</Button>
                <Button onClick={() => openSheet({ kind: "profile", profileId: null, auth: "entra" })}>Sign in with Entra ID</Button>
              </div>
            }
          />
        ) : (
          <section className="flex flex-col gap-1.5 px-4">
            <h2 className="m-0 text-13 font-semibold">Connections</h2>
            <ul aria-label="Saved connections" className="m-0 flex list-none flex-col border-y border-line p-0">
              {profiles.data?.map((p) => (
                <li key={p.id}>
                  <button
                    type="button"
                    aria-label={`${p.name}, ${shortHost(p.host)}${p.production ? ", production" : ""}`}
                    onClick={() => void conn.connect(p.id)}
                    disabled={conn.status === "connecting"}
                    className="flex h-8 w-full items-center gap-2 px-2 text-left text-12 hover:bg-hover disabled:opacity-60"
                  >
                    <LabelSwatch color={p.color} />
                    <span className="font-semibold">{p.name}</span>
                    <span className="truncate text-muted">{shortHost(p.host)}</span>
                    {p.production && <ProdBadge className="ml-auto" />}
                  </button>
                </li>
              ))}
            </ul>
            <div className="pt-1">
              <Button variant="quiet" onClick={() => openSheet({ kind: "profile", profileId: null })}>Add connection…</Button>
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
