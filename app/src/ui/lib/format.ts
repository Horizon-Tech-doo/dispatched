const nf = new Intl.NumberFormat("en-US");

export function formatNumber(n: number): string {
  return nf.format(n);
}

export interface CountQuality {
  approximate?: boolean;
  truncated?: boolean;
}

/** `?` unknown, `~N` counted by peeking, `~N+` when that count hit its cap. */
export function formatCount(n: number | null | undefined, q?: CountQuality): string {
  if (n == null) return "?";
  const s = nf.format(n);
  if (q?.truncated) return `~${s}+`;
  if (q?.approximate) return `~${s}`;
  return s;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)} min ${s % 60} s`;
}

/** Local wall-clock time `HH:MM:SS` for an RFC 3339 timestamp. */
export function formatClock(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleTimeString("en-GB", { hour12: false });
}
