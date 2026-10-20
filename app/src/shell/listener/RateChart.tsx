import { formatBytes, formatNumber } from "@/ui";

const W = 240;
const H = 28;

/** One row of the chart: a bar per second, the newest on the right. */
function Bars({ label, values, text }: { label: string; values: number[]; text(n: number): string }) {
  const peak = Math.max(0, ...values);
  const now = values.at(-1) ?? 0;
  const bw = W / values.length;
  return (
    <div className="flex items-end gap-2">
      <svg role="img" aria-label={`${label}: ${text(now)} now, peak ${text(peak)} in the last minute`} width={W} height={H} viewBox={`0 0 ${W} ${H}`} className="block shrink-0">
        <line x1={0} y1={H - 0.5} x2={W} y2={H - 0.5} className="stroke-line" />
        {values.map((v, i) =>
          v > 0 ? <rect key={i} x={i * bw + 0.5} width={Math.max(1, bw - 1)} y={H - 1 - (v / (peak || 1)) * (H - 2)} height={(v / (peak || 1)) * (H - 2)} className="fill-fg" /> : null,
        )}
      </svg>
      <div className="flex w-28 flex-col text-11 tabular-nums leading-tight">
        <span className="text-fg">{text(now)}</span>
        <span className="text-muted">peak {text(peak)}</span>
      </div>
    </div>
  );
}

/** Messages and bytes per second over the last minute. */
export function RateChart({ messages, bytes }: { messages: number[]; bytes: number[] }) {
  return (
    <div className="flex flex-wrap gap-x-8 gap-y-2" aria-label="Rates">
      <div className="flex flex-col gap-0.5">
        <span className="text-11 text-muted">Messages/s</span>
        <Bars label="Messages per second" values={messages} text={(n) => formatNumber(n)} />
      </div>
      <div className="flex flex-col gap-0.5">
        <span className="text-11 text-muted">Bytes/s</span>
        <Bars label="Bytes per second" values={bytes} text={(n) => formatBytes(n)} />
      </div>
    </div>
  );
}
