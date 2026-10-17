import { describe, expect, it } from "vitest";
import {
  base64ToBytes, detectFormat, formatBody, hexColumns, hexDump, hexHeader, hexRow, hexWidthFor, MAX_LINES, prettyJson, prettyXml, splitLines, utf8OrNull,
} from "./format";

describe("prettyJson", () => {
  it("indents nested values and keeps literals exactly", () => {
    expect(prettyJson('{"a":[1,{"b":12345678901234567890}],"s":"x\\"y, z"}')).toBe(
      '{\n  "a": [\n    1,\n    {\n      "b": 12345678901234567890\n    }\n  ],\n  "s": "x\\"y, z"\n}',
    );
  });
  it("keeps empty containers on one line", () => {
    expect(prettyJson('{"a":{},"b":[]}')).toBe('{\n  "a": {},\n  "b": []\n}');
    expect(prettyJson('{ "a" : {  } , "b":[ ] }')).toBe('{\n  "a": {},\n  "b": []\n}');
  });
  it("returns null for invalid JSON", () => {
    expect(prettyJson("{oops")).toBeNull();
    expect(prettyJson("")).toBeNull();
    expect(prettyJson("   ")).toBeNull();
  });
  it("keeps escapes, unicode and a trailing backslash in strings byte-exact", () => {
    const src = '{"p":"C:\\\\dir\\\\","u":"\\u00e9\\n","e":1e-7,"n":null,"t":true}';
    expect(prettyJson(src)).toBe('{\n  "p": "C:\\\\dir\\\\",\n  "u": "\\u00e9\\n",\n  "e": 1e-7,\n  "n": null,\n  "t": true\n}');
  });
  it("accepts a leading byte order mark and scalar documents", () => {
    expect(prettyJson('\uFEFF{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(prettyJson("42")).toBe("42");
  });
  it("gives up instead of exploding on pathologically deep nesting", () => {
    const deep = "[".repeat(50_000) + "]".repeat(50_000);
    const out = prettyJson(deep);
    expect(out === null || out.length < 8 * deep.length + 70_000).toBe(true);
  });
  it("formats a 4 MiB body in linear time", () => {
    const row = '{"id":12345678901234567890,"name":"item","tags":["a","b"],"ok":true}';
    const text = "[" + Array(Math.ceil((4 * 1024 * 1024) / (row.length + 1))).fill(row).join(",") + "]";
    const t0 = performance.now();
    const out = prettyJson(text)!;
    expect(out).not.toBeNull();
    expect(performance.now() - t0).toBeLessThan(4000);
  });
});

describe("prettyXml", () => {
  it("indents elements and keeps leaf text inline", () => {
    expect(prettyXml('<a x="1"><b>t</b><c/></a>')).toBe('<a x="1">\n  <b>t</b>\n  <c/>\n</a>');
  });
  it("keeps CDATA and comments and the declaration", () => {
    const out = prettyXml('<?xml version="1.0"?><a><!-- note --><b><![CDATA[x < y & z]]></b></a>')!;
    expect(out).toContain("<![CDATA[x < y & z]]>");
    expect(out).toContain("<!-- note -->");
    expect(out.startsWith('<?xml version="1.0"?>\n<a>')).toBe(true);
  });
  it("returns null when tags do not nest", () => {
    expect(prettyXml("<a><b></a></b>")).toBeNull();
    expect(prettyXml("plain text")).toBeNull();
    expect(prettyXml("<a><b></b>")).toBeNull();
  });
  it("keeps a greater-than sign inside an attribute value and handles a doctype", () => {
    expect(prettyXml('<a t="x>y"><b/></a>')).toBe('<a t="x>y">\n  <b/>\n</a>');
    const out = prettyXml('<!DOCTYPE a [<!ENTITY e "v">]><a>&e;</a>')!;
    expect(out).toBe('<!DOCTYPE a [<!ENTITY e "v">]>\n<a>&e;</a>');
  });
  it("returns null for unterminated constructs instead of throwing", () => {
    for (const bad of ["<a><!-- open", "<a><![CDATA[ open", "<a><?pi open", "<a b='open>", "<a", "<", "a < b"]) {
      expect(prettyXml(bad)).toBeNull();
    }
  });
  it("does no quadratic work on many unterminated openers", () => {
    const t0 = performance.now();
    for (const s of ["<!--".repeat(300_000), "<".repeat(1_000_000), "<a ".repeat(300_000), "<![CDATA[".repeat(120_000)]) {
      expect(prettyXml(s)).toBeNull();
    }
    expect(performance.now() - t0).toBeLessThan(3000);
  });
  it("gives up instead of exploding on pathologically deep nesting", () => {
    const deep = "<a>".repeat(60_000) + "</a>".repeat(60_000);
    const out = prettyXml(deep);
    expect(out === null || out.length < 8 * deep.length + 70_000).toBe(true);
  });
  it("formats a 4 MiB body in linear time", () => {
    const row = '<item id="1"><name>n</name><v>2</v><!-- c --></item>';
    const text = "<r>" + row.repeat(Math.ceil((4 * 1024 * 1024) / row.length)) + "</r>";
    const t0 = performance.now();
    expect(prettyXml(text)).not.toBeNull();
    expect(performance.now() - t0).toBeLessThan(4000);
  });
});

describe("detectFormat", () => {
  it("trusts content, not only the content type", () => {
    expect(detectFormat('{"a":1}')).toBe("json");
    expect(detectFormat("<a/>", "text/plain")).toBe("xml");
    expect(detectFormat("not json", "application/json")).toBe("text");
    expect(detectFormat("hello")).toBe("text");
  });
  it("falls back to text for empty, whitespace and look-alike bodies", () => {
    expect(detectFormat("")).toBe("text");
    expect(detectFormat("  \n ", "application/json")).toBe("text");
    expect(detectFormat("[INFO] started")).toBe("text");
    expect(detectFormat("{oops", "application/json")).toBe("text");
    expect(detectFormat("<html><br></html>", "text/xml")).toBe("text");
  });
  it("detects after leading whitespace and with structured suffix types", () => {
    expect(detectFormat('\n  {"a":1}')).toBe("json");
    expect(detectFormat('{"a":1}', "application/vnd.api+json; charset=utf-8")).toBe("json");
    expect(detectFormat("  <a><b/></a>", "application/atom+xml")).toBe("xml");
  });
  it("formatBody returns the pretty text with the format, or the input unchanged", () => {
    expect(formatBody('{"a":1}')).toEqual({ format: "json", text: '{\n  "a": 1\n}', formatted: true });
    expect(formatBody("hello")).toEqual({ format: "text", text: "hello", formatted: false });
  });
  it("formatBody says when a body was too large to format and returns the raw text", () => {
    const deep = "[".repeat(50_000) + "]".repeat(50_000);
    expect(formatBody(deep, "application/json")).toEqual({ format: "text", text: deep, formatted: false, reason: "too-large" });
    const deepXml = "<a>".repeat(60_000) + "</a>".repeat(60_000);
    expect(formatBody(deepXml)).toEqual({ format: "text", text: deepXml, formatted: false, reason: "too-large" });
    expect(formatBody("{oops", "application/json").reason).toBeUndefined();
  });
});

describe("bytes", () => {
  it("decodes base64 and UTF-8, refusing invalid UTF-8", () => {
    const utf8 = new TextEncoder().encode("héllo");
    expect(utf8OrNull(utf8)).toBe("héllo");
    expect(Array.from(base64ToBytes(btoa("Hi"))!)).toEqual([0x48, 0x69]);
    expect(utf8OrNull(new Uint8Array([0xff, 0xfe]))).toBeNull();
    expect(utf8OrNull(new Uint8Array())).toBe("");
  });
  it("base64ToBytes returns null instead of throwing on invalid input", () => {
    expect(base64ToBytes("not base64 !!")).toBeNull();
    expect(base64ToBytes("a")).toBeNull();
    expect(Array.from(base64ToBytes("")!)).toEqual([]);
  });
  it("hexDump shows non-printables as dots", () => {
    const lines = hexDump(new Uint8Array([0x7b, 0x00, 0x41, 0xff]));
    expect(lines).toEqual(["000000  7b 00 41 ff                                       |{.A.|"]);
    expect(hexDump(new Uint8Array(40))).toHaveLength(3);
    expect(hexDump(new Uint8Array(40), 16, 32)[0].startsWith("000010")).toBe(true);
  });
  it("hexDump aligns a full row and copes with empty and out-of-range input", () => {
    const full = hexDump(Uint8Array.from({ length: 16 }, (_, i) => i + 0x30))[0];
    expect(full).toBe("000000  30 31 32 33 34 35 36 37  38 39 3a 3b 3c 3d 3e 3f  |0123456789:;<=>?|");
    expect(hexDump(new Uint8Array())).toEqual([]);
    expect(hexDump(new Uint8Array(4), 100, 200)).toEqual([]);
    expect(hexDump(new Uint8Array(4), 0, 1000)).toHaveLength(1);
  });
  it("hexDump holds 8 bytes per row when asked, with the hex column as wide as the title", () => {
    const bytes = Uint8Array.from({ length: 20 }, (_, i) => i + 0x30);
    const rows = hexDump(bytes, 0, bytes.length, 8);
    expect(rows).toHaveLength(3);
    expect(rows[0]).toBe("000000  30 31 32 33 34 35 36 37  |01234567|");
    expect(rows[2]).toBe(`000010  ${"40 41 42 43".padEnd(hexColumns(8))}  |@ABC|`);
    expect(hexHeader(8)).toBe("00 01 02 03 04 05 06 07");
    expect(hexHeader(16)).toBe("00 01 02 03 04 05 06 07  08 09 0a 0b 0c 0d 0e 0f");
    for (const w of [8, 16] as const) expect(hexRow(bytes, 0, 20, w).hex).toHaveLength(hexColumns(w));
  });
  it("offsets keep six digits and grow only past them", () => {
    expect(hexRow(new Uint8Array(4), 0).offset).toBe("000000");
    expect(hexRow(new Uint8Array(0x400000 + 16), 0x400000).offset).toBe("400000");
  });
  it("hexWidthFor chooses 16 bytes only when offset, hex and text fit", () => {
    // 72 columns of 6.6 px and 52 px of padding: 527.2 px.
    expect(hexWidthFor(600, 6.6)).toBe(16);
    expect(hexWidthFor(528, 6.6)).toBe(16);
    expect(hexWidthFor(527, 6.6)).toBe(8);
    expect(hexWidthFor(300, 6.6)).toBe(8);
    // Unmeasured (not laid out yet): the wide layout.
    expect(hexWidthFor(0, 6.6)).toBe(16);
    expect(hexWidthFor(600, 0)).toBe(16);
  });
});

describe("splitLines", () => {
  it("splits on LF and CRLF like a text editor", () => {
    expect(splitLines("a\r\nb\nc").lines).toEqual(["a", "b", "c"]);
    expect(splitLines("").lines).toEqual([""]);
    expect(splitLines("a\n").lines).toEqual(["a", ""]);
  });
  it("keeps at most max lines and counts the rest", () => {
    const r = splitLines("1\n2\n3\n4\n5", 2);
    expect(r).toEqual({ lines: ["1", "2"], more: 3 });
    expect(MAX_LINES).toBeGreaterThan(100_000);
  });
});
