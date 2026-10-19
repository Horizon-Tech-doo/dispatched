import type { PropType, PropValue } from "@/engine";

/** The types a property can be sent as, in menu order. */
export const PROP_TYPES: PropType[] = ["string", "int", "long", "double", "bool", "datetime", "timespan", "guid"];

const INT = /^[+-]?\d+$/;
const fitsInt = (t: string, bits: 32 | 64) => {
  if (!INT.test(t)) return false;
  const n = BigInt(t);
  const max = (1n << BigInt(bits - 1)) - 1n;
  return n >= -max - 1n && n <= max;
};
const NANOS = 1_000_000_000n;

/** Digits with an optional fraction, times `unit` seconds, in nanoseconds (as core `decimal_nanos`). */
function decimalNanos(num: string, unit: bigint): bigint | null {
  const [whole, frac = ""] = num.split(".", 2);
  if ((whole === "" && frac === "") || !/^\d*$/.test(whole) || !/^\d*$/.test(frac)) return null;
  const f = frac.slice(0, 9).padEnd(9, "0");
  return BigInt(whole || "0") * unit * NANOS + (BigInt(f) * unit * NANOS) / NANOS;
}

function units(t: string): bigint | null {
  let total = 0n;
  let rest = t;
  while (rest) {
    const m = /^([\d.]*)([dhmsDHMS])/.exec(rest);
    if (!m) return null;
    const unit = { d: 86_400n, h: 3_600n, m: 60n, s: 1n }[m[2].toLowerCase() as "d"];
    const n = decimalNanos(m[1], unit);
    if (n === null) return null;
    total += n;
    rest = rest.slice(m[0].length);
  }
  return total;
}

function iso(t: string): bigint | null {
  const upper = t.toUpperCase();
  const [date, time] = upper.includes("T") ? (upper.split("T", 2) as [string, string]) : [upper, undefined];
  if (!date && !time) return null;
  let total = 0n;
  for (const [part, us] of [
    [date, [["W", 604_800n], ["D", 86_400n]]],
    [time ?? "", [["H", 3_600n], ["M", 60n], ["S", 1n]]],
  ] as const) {
    let rest: string = part;
    let last = 0;
    while (rest) {
      const m = /^([\d.]*)([A-Z])/.exec(rest);
      if (!m) return null;
      const idx = us.findIndex(([c]) => c === m[2]);
      if (idx < last) return null;
      last = idx + 1;
      const n = decimalNanos(m[1], us[idx][1]);
      if (n === null) return null;
      total += n;
      rest = rest.slice(m[0].length);
    }
  }
  return total;
}

function clock(t: string): bigint | null {
  const parts = t.split(":");
  if (parts.length !== 3) return null;
  const [d, h] = parts[0].includes(".") ? parts[0].split(".", 2) : ["0", parts[0]];
  if (!/^\d+$/.test(d) || !/^\d+$/.test(h) || !/^\d+$/.test(parts[1])) return null;
  const hours = Number(h);
  const minutes = Number(parts[1]);
  if (hours > 23 || minutes > 59) return null;
  const seconds = decimalNanos(parts[2], 1n);
  if (seconds === null || seconds >= 60n * NANOS) return null;
  return BigInt(d) * 86_400n * NANOS + BigInt(hours * 3600 + minutes * 60) * NANOS + seconds;
}

/** A duration as core `parse_nanos` reads it: `90s`, `1h30m`, ISO 8601 (`PT1H30M`) or `[d.]hh:mm:ss`. Null when not one. */
export function durationNanos(text: string): bigint | null {
  let t = text.trim();
  let neg = false;
  if (t.startsWith("-")) {
    neg = true;
    t = t.slice(1);
  } else if (t.startsWith("+")) t = t.slice(1);
  if (!t) return null;
  const n = /^[Pp]/.test(t) ? iso(t.slice(1)) : t.includes(":") ? clock(t) : units(t);
  return n === null ? null : neg ? -n : n;
}

/** A time to live: a positive duration. Null when fine. */
export function checkTtl(text: string): string | null {
  if (!text.trim()) return null;
  const n = durationNanos(text);
  if (n === null) return "Use a duration such as 90s, 15m, 2h, 7d or PT1H30M.";
  if (n <= 0n) return "Use a duration above zero.";
  return null;
}

/** As `uuid::Uuid::parse_str`: hyphenated, simple (32 digits), braced or `urn:uuid:`. */
const GUID = /^(?:urn:uuid:)?(?:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[0-9a-f]{32}|\{[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\})$/i;
const RFC3339 = /^\d{4}-\d{2}-\d{2}[Tt ]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/;

/** Whether `text` is a valid value of `type`, by the rules of core `TypedValue::parse`. Null when it is. */
export function checkValue(type: PropType, text: string): string | null {
  const t = text.trim();
  switch (type) {
    case "string":
      return null;
    case "int":
      if (fitsInt(t, 32)) return null;
      return fitsInt(t, 64) ? `${t} does not fit an int (32 bits). Use long.` : `${t || "This"} is not a valid int.`;
    case "long":
      return fitsInt(t, 64) ? null : `${t || "This"} is not a valid long.`;
    case "double": {
      const n = t === "" ? NaN : Number(t);
      return Number.isFinite(n) ? null : `${t || "This"} is not a valid double.`;
    }
    case "bool":
      return /^(true|false)$/i.test(t) ? null : `${t || "This"} is not a valid bool. Use true or false.`;
    case "datetime":
      return RFC3339.test(t) && !Number.isNaN(Date.parse(t)) ? null : `${t || "This"} is not a valid datetime. Use an RFC 3339 time, such as 2026-10-17T12:00:00Z.`;
    case "timespan":
      return durationNanos(t) !== null ? null : `${t || "This"} is not a valid timespan. Use 1h30m, PT1H30M or [d.]hh:mm:ss.`;
    case "guid":
      return GUID.test(t) ? null : `${t || "This"} is not a valid guid.`;
  }
}

/** Message ids from `base`, as core `MessageIdSource::Increment`: trailing digits count up and keep their zero padding;
 *  without trailing digits, `base`, then `base-1`, `base-2`, … */
export function incrementId(base: string, index: number): string {
  const m = /(\d+)$/.exec(base);
  if (m) {
    const next = (BigInt(m[1]) + BigInt(index)).toString();
    return `${base.slice(0, m.index)}${next.padStart(m[1].length, "0")}`;
  }
  return index === 0 ? base : `${base}-${index}`;
}

/** How a received property is sent again (Clone & resend, Decision 33): as one of the form's types when that keeps
 *  its value, with the original type named when it changes; `other`, `binary` and `null` cannot be sent from a form. */
export function copyOf(v: PropValue): { type: PropType; value: string; was?: string } | { locked: string } {
  switch (v.type) {
    case "string":
    case "int":
    case "long":
    case "double":
    case "bool":
      return { type: v.type, value: String(v.value) };
    case "timespan":
      return { type: "timespan", value: v.value };
    case "timestamp":
      return { type: "datetime", value: v.value };
    case "uuid":
      return { type: "guid", value: v.value };
    case "symbol":
      return { type: "string", value: v.value, was: "symbol" };
    case "byte":
    case "short":
    case "ubyte":
    case "ushort":
      return { type: "int", value: String(v.value), was: v.type };
    case "uint":
      return { type: "long", value: String(v.value), was: v.type };
    case "ulong":
      return Number.isSafeInteger(v.value) ? { type: "long", value: String(v.value), was: v.type } : { locked: "Its value (ulong) is too large to send from here." };
    case "float":
      return { type: "double", value: String(v.value), was: "float" };
    default:
      return { locked: `Its type (${v.type}) can't be sent from here.` };
  }
}
