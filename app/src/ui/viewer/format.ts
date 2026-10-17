export type BodyFormat = "json" | "xml" | "text";
/** Display limit per line; longer lines are cut (copy and export keep everything). */
export const MAX_LINE = 20_000;
/**
 * Display limit in lines. Browsers cap an element's height (about 33 million px, less in some engines), so a body
 * of millions of tiny lines would scroll wrongly; past this the view says how many lines are left out.
 */
export const MAX_LINES = 500_000;

/** Formatted output may grow to this many times the input (plus slack) before formatting gives up and the raw text shows. */
const budgetFor = (inputLength: number) => inputLength * 8 + 65_536;

/** Internal result of the formatters: the text is valid, but indenting it would exceed the size budget. */
const OVER = Symbol("over budget");

const isSpace = (c: number) => c === 32 || c === 10 || c === 13 || c === 9;

/**
 * Re-indent JSON by walking its tokens, so numbers and string escapes stay byte-exact.
 * `null` when the text is not JSON, or when indenting would blow up (extreme nesting).
 */
export function prettyJson(text: string): string | null {
  const r = indentJson(text);
  return r === OVER ? null : r;
}

function indentJson(text: string): string | typeof OVER | null {
  const src = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  try {
    JSON.parse(src);
  } catch {
    return null;
  }
  // From here the text is valid JSON, so the scan below needs no error handling.
  const n = src.length;
  const budget = budgetFor(n);
  const out: string[] = [];
  let size = 0;
  let depth = 0;
  let over = false;
  const push = (s: string) => {
    out.push(s);
    size += s.length;
  };
  const newline = () => {
    if (size + 1 + depth * 2 > budget) over = true;
    else push("\n" + "  ".repeat(depth));
  };
  let i = 0;
  while (i < n && !over) {
    const c = src.charCodeAt(i);
    if (isSpace(c)) {
      i++;
    } else if (c === 34) {
      let j = i + 1;
      while (j < n) {
        const q = src.charCodeAt(j);
        if (q === 92) j += 2;
        else if (q === 34) break;
        else j++;
      }
      push(src.slice(i, j + 1));
      i = j + 1;
    } else if (c === 123 || c === 91) {
      let j = i + 1;
      while (j < n && isSpace(src.charCodeAt(j))) j++;
      if (src.charCodeAt(j) === (c === 123 ? 125 : 93)) {
        push(c === 123 ? "{}" : "[]");
        i = j + 1;
      } else {
        push(c === 123 ? "{" : "[");
        depth++;
        newline();
        i++;
      }
    } else if (c === 125 || c === 93) {
      depth--;
      newline();
      push(c === 125 ? "}" : "]");
      i++;
    } else if (c === 44) {
      push(",");
      newline();
      i++;
    } else if (c === 58) {
      push(": ");
      i++;
    } else {
      let j = i + 1;
      while (j < n) {
        const q = src.charCodeAt(j);
        if (q === 44 || q === 125 || q === 93 || q === 58 || isSpace(q)) break;
        j++;
      }
      push(src.slice(i, j));
      i = j;
    }
  }
  return over ? OVER : out.join("");
}

/** Index just past the `>` that closes the tag starting at `i`, honouring quoted values and a DOCTYPE's `[ ]`. -1 when it never closes. */
function tagEnd(s: string, i: number): number {
  const declaration = s.charCodeAt(i + 1) === 33;
  let quote = 0;
  let bracket = 0;
  for (let j = i + 1; j < s.length; j++) {
    const c = s.charCodeAt(j);
    if (quote) {
      if (c === quote) quote = 0;
    } else if (c === 34 || c === 39) quote = c;
    else if (declaration && c === 91) bracket++;
    else if (declaration && c === 93) bracket--;
    else if (c === 62 && bracket <= 0) return j + 1;
  }
  return -1;
}

/** Split XML into tags, comments, CDATA, processing instructions and text, in one pass. `null` when a construct never closes. */
function tokenizeXml(s: string): string[] | null {
  const tokens: string[] = [];
  const n = s.length;
  let i = 0;
  while (i < n) {
    if (s.charCodeAt(i) !== 60) {
      const j = s.indexOf("<", i);
      const end = j < 0 ? n : j;
      tokens.push(s.slice(i, end));
      i = end;
      continue;
    }
    let end: number;
    if (s.startsWith("<!--", i)) {
      const e = s.indexOf("-->", i + 4);
      end = e < 0 ? -1 : e + 3;
    } else if (s.startsWith("<![CDATA[", i)) {
      const e = s.indexOf("]]>", i + 9);
      end = e < 0 ? -1 : e + 3;
    } else if (s.startsWith("<?", i)) {
      const e = s.indexOf("?>", i + 2);
      end = e < 0 ? -1 : e + 2;
    } else {
      end = tagEnd(s, i);
    }
    if (end < 0) return null;
    tokens.push(s.slice(i, end));
    i = end;
  }
  return tokens;
}

const NAME_START = /[A-Za-z_:À-￿]/;
const TAG_NAME = /^[^\s>/]+/;
const isCdata = (t: string) => t.startsWith("<![CDATA[");

/** Indent XML by its tags; `null` when there are no tags, they do not nest, or indenting would blow up. */
export function prettyXml(text: string): string | null {
  const r = indentXml(text);
  return r === OVER ? null : r;
}

function indentXml(text: string): string | typeof OVER | null {
  const src = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  const tokens = tokenizeXml(src);
  if (!tokens || !tokens.some((t) => t.startsWith("<") && !t.startsWith("<?"))) return null;
  const budget = budgetFor(src.length);
  const lines: string[] = [];
  const stack: string[] = [];
  let size = 0;
  let over = false;
  const add = (s: string) => {
    const width = stack.length * 2 + s.length + 1;
    if (size + width > budget) over = true;
    else {
      size += width;
      lines.push("  ".repeat(stack.length) + s);
    }
  };
  for (let i = 0; i < tokens.length && !over; i++) {
    const t = tokens[i];
    if (!t.startsWith("<")) {
      const s = t.trim();
      if (s) add(s);
      continue;
    }
    if (t.startsWith("</")) {
      if (!NAME_START.test(t[2] ?? "")) return null;
      const name = t.slice(2, -1).trim();
      if (stack.pop() !== name) return null;
      add(t);
      continue;
    }
    if (t.startsWith("<!") || t.startsWith("<?")) {
      add(t);
      continue;
    }
    if (!NAME_START.test(t[1] ?? "")) return null;
    if (t.endsWith("/>")) {
      add(t);
      continue;
    }
    const name = TAG_NAME.exec(t.slice(1))![0];
    const next = tokens[i + 1];
    // <b>leaf</b> and <b><![CDATA[x]]></b> stay on one line.
    if (next !== undefined && (!next.startsWith("<") || isCdata(next))) {
      const close = tokens[i + 2];
      if (close === `</${name}>`) {
        add(t + (isCdata(next) ? next : next.trim()) + close);
        i += 2;
        continue;
      }
    }
    add(t);
    stack.push(name);
  }
  if (over) return OVER;
  return stack.length === 0 ? lines.join("\n") : null;
}

export interface FormattedBody {
  format: BodyFormat;
  /** The display text: re-indented when `formatted`, the input unchanged otherwise. */
  text: string;
  /** True when `text` was re-indented. */
  formatted: boolean;
  /** Set when the body is JSON or XML that was too large to indent (extreme nesting): `text` is the raw input. */
  reason?: "too-large";
}

/** The format of a body and its display text in one pass (so a 4 MiB body is parsed once). Content wins over the content type. */
export function formatBody(text: string, contentType?: string | null): FormattedBody {
  let at = 0;
  while (at < text.length && (isSpace(text.charCodeAt(at)) || text.charCodeAt(at) === 0xfeff)) at++;
  const head = text[at];
  const ct = (contentType ?? "").toLowerCase();
  if (head === "{" || head === "[" || ct.includes("json")) {
    const pretty = indentJson(text);
    if (pretty === OVER) return { format: "text", text, formatted: false, reason: "too-large" };
    if (pretty !== null) return { format: "json", text: pretty, formatted: true };
  }
  if (head === "<" || ct.includes("xml")) {
    const pretty = indentXml(text);
    if (pretty === OVER) return { format: "text", text, formatted: false, reason: "too-large" };
    if (pretty !== null) return { format: "xml", text: pretty, formatted: true };
  }
  return { format: "text", text, formatted: false };
}

export function detectFormat(text: string, contentType?: string | null): BodyFormat {
  return formatBody(text, contentType).format;
}

/** Decode standard base64; `null` when the input is not valid base64. */
export function base64ToBytes(b64: string): Uint8Array | null {
  let raw: string;
  try {
    raw = atob(b64);
  } catch {
    return null;
  }
  const out = new Uint8Array(raw.length);
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i);
  return out;
}

export function utf8OrNull(bytes: Uint8Array): string | null {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
}

const HEX = Array.from({ length: 256 }, (_, b) => b.toString(16).padStart(2, "0"));

/** Bytes per hex row: 16 when the view is wide enough, else 8. */
export type HexWidth = 8 | 16;
/** The widest row. */
export const HEX_ROW = 16;
/** Offsets are at least this many hex digits (a 4 MiB body needs six). */
export const HEX_OFFSET_DIGITS = 6;

/** Characters of the hex column for `width` bytes: pairs joined by a space, a second space between the two groups of 8. */
export const hexColumns = (width: HexWidth) => (width === 16 ? 48 : 23);

/** The column titles `00 01 .. 0f` for `width` bytes, as wide as `hexColumns`. */
export function hexHeader(width: HexWidth): string {
  const cols = Array.from({ length: width }, (_, i) => i.toString(16).padStart(2, "0"));
  return width === 16 ? `${cols.slice(0, 8).join(" ")}  ${cols.slice(8).join(" ")}` : cols.join(" ");
}

/** The narrowest wide-enough layout: 16 bytes per row when offset, hex and text columns (and their padding) fit
 *  in `available` pixels for a mono character `charWidth` pixels wide, else 8. Below even that the view scrolls. */
export function hexWidthFor(available: number, charWidth: number): HexWidth {
  if (!(available > 0) || !(charWidth > 0)) return 16;
  // offset + hex + "|text|", and the paddings the view puts around the three columns (12 + 12 + 12 + 16 px).
  const need = (HEX_OFFSET_DIGITS + hexColumns(16) + HEX_ROW + 2) * charWidth + 52;
  return available >= need ? 16 : 8;
}

export interface HexRow {
  offset: string;
  /** Padded to the width of a full row so the text column lines up. */
  hex: string;
  /** `|text|` with non-printable bytes as dots. */
  ascii: string;
}

/** One row of up to `width` bytes from `at`, never reading past `end`. */
export function hexRow(bytes: Uint8Array, at: number, end = bytes.length, width: HexWidth = 16): HexRow {
  const stop = Math.min(at + width, end, bytes.length);
  const cells: string[] = [];
  let ascii = "";
  for (let i = at; i < stop; i++) {
    const b = bytes[i];
    cells.push(HEX[b]);
    ascii += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : ".";
  }
  const left = cells.slice(0, 8).join(" ");
  const right = cells.slice(8).join(" ");
  return {
    offset: at.toString(16).padStart(HEX_OFFSET_DIGITS, "0"),
    hex: (right ? `${left}  ${right}` : left).padEnd(hexColumns(width), " "),
    ascii: `|${ascii}|`,
  };
}

/** `000010  7b 22 69 …  |{"i…|` rows for bytes [start, end). */
export function hexDump(bytes: Uint8Array, start = 0, end = bytes.length, width: HexWidth = 16): string[] {
  const rows: string[] = [];
  for (let at = Math.max(0, start); at < Math.min(end, bytes.length); at += width) {
    const r = hexRow(bytes, at, end, width);
    rows.push(`${r.offset}  ${r.hex}  ${r.ascii}`);
  }
  return rows;
}

/**
 * Split on LF or CRLF in one linear pass, keeping at most `max` lines and counting the rest.
 * Like `split("\n")`, text that ends in a newline ends in an empty line.
 */
export function splitLines(text: string, max = MAX_LINES): { lines: string[]; more: number } {
  const lines: string[] = [];
  let more = 0;
  let at = 0;
  for (;;) {
    const nl = text.indexOf("\n", at);
    const stop = nl < 0 ? text.length : nl;
    if (lines.length < max) lines.push(text.slice(at, stop > at && text.charCodeAt(stop - 1) === 13 ? stop - 1 : stop));
    else more++;
    if (nl < 0) break;
    at = nl + 1;
  }
  return { lines, more };
}
