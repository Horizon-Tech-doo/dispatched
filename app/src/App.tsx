// M0 plumbing proof: connect → list queues → peek / peek DLQ.
// No visual design here; the design system is M1 (impeccable + taste).
import { useState } from "react";
import * as api from "./api";

export default function App() {
  const [connectionString, setConnectionString] = useState("");
  const [info, setInfo] = useState<api.ConnectionInfo | null>(null);
  const [queues, setQueues] = useState<api.QueueInfo[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [subQueue, setSubQueue] = useState<api.SubQueue>("active");
  const [messages, setMessages] = useState<api.PeekedMessage[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function guarded<T>(work: () => Promise<T>): Promise<T | undefined> {
    setBusy(true);
    setError(null);
    try {
      return await work();
    } catch (e) {
      const err = e as api.CommandError;
      setError(err?.message ?? String(e));
    } finally {
      setBusy(false);
    }
  }

  const connect = () =>
    guarded(async () => {
      const i = await api.connectConnectionString(connectionString);
      setInfo(i);
      setQueues(await api.listQueues());
    });

  const open = (name: string, sq: api.SubQueue) =>
    guarded(async () => {
      setSelected(name);
      setSubQueue(sq);
      setMessages(await api.peek(name, sq));
    });

  return (
    <main>
      <h1>Dispatched</h1>
      {!info ? (
        <form onSubmit={(e) => { e.preventDefault(); void connect(); }}>
          <label>
            Connection string{" "}
            <input type="password" value={connectionString} onChange={(e) => setConnectionString(e.target.value)} size={80} />
          </label>{" "}
          <button type="submit" disabled={busy || !connectionString}>Connect</button>
        </form>
      ) : (
        <p>
          Connected to <strong>{info.host}</strong> {info.emulator ? "(emulator)" : ""} via {info.auth}.{" "}
          <button onClick={() => guarded(async () => { await api.disconnect(); setInfo(null); setQueues([]); setMessages([]); })}>Disconnect</button>
        </p>
      )}
      {error && <p role="alert">Error: {error}</p>}
      {queues.length > 0 && (
        <table>
          <thead><tr><th>Queue</th><th>Active</th><th>DLQ</th><th></th></tr></thead>
          <tbody>
            {queues.map((q) => (
              <tr key={q.name}>
                <td>{q.name}</td>
                <td>{q.counts?.active ?? "?"}</td>
                <td>{q.counts?.dead_letter ?? "?"}</td>
                <td>
                  <button onClick={() => open(q.name, "active")} disabled={busy}>Peek</button>{" "}
                  <button onClick={() => open(q.name, "dead-letter")} disabled={busy}>Peek DLQ</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {selected && (
        <section>
          <h2>{selected} {subQueue === "active" ? "" : "— dead-letter"}</h2>
          <table>
            <thead><tr><th>Seq</th><th>Enqueued</th><th>Message ID</th><th>Deliveries</th><th>Reason</th><th>Body</th></tr></thead>
            <tbody>
              {messages.map((m) => (
                <tr key={m.sequence_number}>
                  <td>{m.sequence_number}</td>
                  <td>{m.enqueued_time}</td>
                  <td>{m.message_id}</td>
                  <td>{m.delivery_count}</td>
                  <td>{m.dead_letter_reason}</td>
                  <td><code>{api.bodyPreview(m.body).slice(0, 200)}</code></td>
                </tr>
              ))}
            </tbody>
          </table>
          {messages.length === 0 && <p>No messages.</p>}
        </section>
      )}
    </main>
  );
}
