/** How the app writes a length of time, for both time to live on a message (milliseconds) and the durations the
 *  service reports on an entity (ISO 8601): `14 d`, `1 d 2 h`, `1 h 30 min`, `45 s`. */
const NEVER_MS = 1e14;
export const NEVER = "Never";
const UNITS: [number, string][] = [[86_400_000, "d"], [3_600_000, "h"], [60_000, "min"], [1000, "s"]];

/** `maxParts` caps the units shown (the largest first); `exactSeconds` keeps fractions of a second. */
function describe(ms: number, maxParts: number, exactSeconds: boolean): string {
  if (ms >= NEVER_MS) return NEVER;
  const parts: string[] = [];
  let rest = ms;
  for (const [size, unit] of UNITS) {
    if (exactSeconds && unit === "s") {
      if (rest > 0) parts.push(`${Number((rest / 1000).toFixed(3))} s`);
      break;
    }
    const n = Math.floor(rest / size);
    if (n > 0 && parts.length < maxParts) parts.push(`${n} ${unit}`);
    rest -= n * size;
  }
  if (parts.length) return parts.join(" ");
  return exactSeconds ? "0 s" : `${ms} ms`;
}

/** Time to live in milliseconds: the two largest whole units; the service's maximum (TimeSpan.MaxValue) as "Never". */
export function formatTtl(ms: number | null): string {
  return ms == null ? "" : describe(ms, 2, false);
}

/** `PT1M` becomes `1 min`, `P14D` becomes `14 d`, and .NET's TimeSpan.MaxValue (`P10675199D...`) is "Never".
 *  Null for text that is not a day-and-time ISO 8601 duration. */
export function formatIsoDuration(v: string): string | null {
  const m = /^P(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+(?:\.\d+)?)S)?)?$/.exec(v);
  if (!m || v === "P" || v.endsWith("T")) return null;
  const [d, h, min, s] = [m[1], m[2], m[3], m[4]].map((x) => (x ? Number(x) : 0));
  return describe(d * 86_400_000 + h * 3_600_000 + min * 60_000 + s * 1000, Infinity, true);
}
