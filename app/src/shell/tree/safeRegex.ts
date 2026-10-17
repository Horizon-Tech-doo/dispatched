/** A small regular-expression matcher for highlight rules that cannot hang.
 *
 *  A rule's pattern is typed by the user and run against every entity name on every render, so a pattern such as
 *  `^(a+)+$` must not be able to stall the window the way it can in the browser's own backtracking engine. This
 *  matcher compiles to a Thompson automaton and runs all of its states side by side, so a match costs at most
 *  (name length x program size) steps whatever the pattern says.
 *
 *  Supported (a subset of JavaScript's syntax, matched case-insensitively and anywhere in the name): literals,
 *  `.`, `^`, `$`, `\b`, `\B`, `[...]` and `[^...]` with ranges, `\d \D \w \W \s \S`, groups `( )` and `(?: )`,
 *  alternation `|`, and `* + ?` and `{n}`, `{n,}`, `{n,m}` (counts up to 100; a trailing `?` is accepted and ignored,
 *  since only whether the name matches is asked). Look-ahead, look-behind, back-references and named groups are
 *  refused with an error. */

export const MAX_PATTERN = 200;
/** Names are far shorter (260 characters at most); anything beyond is not examined. */
const MAX_NAME = 512;
const MAX_REPEAT = 100;
/** Compiled size, in instructions and in nodes emitted (a repeat of an empty group emits nodes but no instructions). */
const MAX_PROGRAM = 1000;
const MAX_DEPTH = 50;

export type Compiled = { test(name: string): boolean };
export type PatternResult = Compiled | { error: string };

type Test = (c: string) => boolean;
type Node =
  | { t: "set"; test: Test }
  | { t: "assert"; kind: "start" | "end" | "word" | "notword" }
  | { t: "alt"; alts: Node[][] }
  | { t: "rep"; node: Node; min: number; max: number };

class PatternError extends Error {}
const fail = (message: string): never => {
  throw new PatternError(message);
};

const isDigit: Test = (c) => c >= "0" && c <= "9";
const isWord: Test = (c) => (c >= "a" && c <= "z") || (c >= "A" && c <= "Z") || isDigit(c) || c === "_";
const isSpace: Test = (c) => /^\s$/u.test(c);
const not = (t: Test): Test => (c) => !t(c);
/** `d w s` and their upper-case negations. */
const CLASS_ESCAPES: Record<string, Test> = { d: isDigit, D: not(isDigit), w: isWord, W: not(isWord), s: isSpace, S: not(isSpace) };
const CONTROL_ESCAPES: Record<string, string> = { n: "\n", r: "\r", t: "\t", f: "\f", v: "\v", "0": "\0" };
/** Escapes with a meaning this matcher does not give them: refused, so a rule never silently means something else. */
const UNSUPPORTED_ESCAPES = new Set(["1", "2", "3", "4", "5", "6", "7", "8", "9", "k", "p", "P", "c", "u", "x"]);

/** JavaScript's case-insensitive canonical form (non-unicode mode): the upper-case letter when that is one character,
 *  and never a non-ASCII character folded into ASCII (so `ß`, `ı` and the Kelvin sign keep their own identity). */
const canon = (ch: string): string => {
  const up = ch.toUpperCase();
  if (up.length !== ch.length) return ch;
  return ch.charCodeAt(0) >= 128 && up.charCodeAt(0) < 128 ? ch : up;
};

/** A test answered from a table for ASCII, where names are almost always written: a class of a hundred characters is
 *  then no slower than a single one. Other characters go through `test`. */
const fast = (test: Test): Test => {
  const table = new Uint8Array(128);
  for (let i = 0; i < 128; i++) table[i] = test(String.fromCharCode(i)) ? 1 : 0;
  return (c) => {
    const code = c.charCodeAt(0);
    return code < 128 && c.length === 1 ? table[code] === 1 : test(c);
  };
};

const literal = (ch: string): Test => {
  const want = canon(ch);
  return fast((c) => c === ch || canon(c) === want);
};

class Parser {
  private pos = 0;
  private readonly p: string[];
  constructor(pattern: string) {
    this.p = Array.from(pattern);
  }

  parse(): Node {
    const node = this.alt(0);
    if (this.pos < this.p.length) fail("a ) has no matching (");
    return node;
  }

  private peek(): string | undefined {
    return this.p[this.pos];
  }

  private alt(depth: number): Node {
    if (depth > MAX_DEPTH) fail("the groups are nested too deeply");
    const alts: Node[][] = [this.seq(depth)];
    while (this.peek() === "|") {
      this.pos++;
      alts.push(this.seq(depth));
    }
    return { t: "alt", alts };
  }

  private seq(depth: number): Node[] {
    const out: Node[] = [];
    for (let c = this.peek(); c !== undefined && c !== "|" && c !== ")"; c = this.peek()) {
      const atom = this.atom(depth);
      out.push(this.quantified(atom));
    }
    return out;
  }

  /** `{n}`, `{n,}` or `{n,m}` at the cursor: its bounds and length, else null (a `{` that is only a character). */
  private braces(): { min: number; max: number; length: number } | null {
    if (this.peek() !== "{") return null;
    const m = /^\{(\d+)(?:(,)(\d*))?\}/.exec(this.p.slice(this.pos, this.pos + 16).join(""));
    if (!m) return null;
    const min = Number(m[1]);
    const max = m[2] === undefined ? min : m[3] === "" ? Infinity : Number(m[3]);
    if (min > MAX_REPEAT || (max !== Infinity && max > MAX_REPEAT)) fail(`repeat counts go up to ${MAX_REPEAT}`);
    if (max < min) fail("the repeat counts are out of order");
    return { min, max, length: m[0].length };
  }

  private quantified(atom: Node): Node {
    let node = atom;
    const c = this.peek();
    let q: { min: number; max: number } | null = null;
    if (c === "*") q = { min: 0, max: Infinity };
    else if (c === "+") q = { min: 1, max: Infinity };
    else if (c === "?") q = { min: 0, max: 1 };
    if (q) this.pos++;
    else {
      const b = this.braces();
      if (b) {
        q = b;
        this.pos += b.length;
      }
    }
    if (q) {
      if (atom.t === "assert") fail(`nothing to repeat before ${c}`);
      node = { t: "rep", node: atom, min: q.min, max: q.max };
      if (this.peek() === "?") this.pos++; // lazy: the same names match
      const next = this.peek();
      if (next === "*" || next === "+" || next === "?" || this.braces()) fail(`nothing to repeat before ${next}`);
    }
    return node;
  }

  private atom(depth: number): Node {
    const c = this.p[this.pos++];
    switch (c) {
      case "(": {
        if (this.peek() === "?") {
          if (this.p[this.pos + 1] !== ":") fail("look-ahead, look-behind and named groups aren't supported");
          this.pos += 2;
        }
        const inner = this.alt(depth + 1);
        if (this.peek() !== ")") fail("a ( is never closed");
        this.pos++;
        return inner;
      }
      case "[":
        return this.charClass();
      case ".":
        return { t: "set", test: (ch) => ch !== "\n" && ch !== "\r" && ch !== "\u2028" && ch !== "\u2029" };
      case "^":
        return { t: "assert", kind: "start" };
      case "$":
        return { t: "assert", kind: "end" };
      case "*":
      case "+":
      case "?":
        return fail(`nothing to repeat before ${c}`);
      case "{":
        this.pos--;
        if (this.braces()) fail("nothing to repeat before {");
        this.pos++;
        return { t: "set", test: literal("{") };
      case "\\":
        return this.escape();
      default:
        return { t: "set", test: literal(c) };
    }
  }

  private escape(): Node {
    const c = this.p[this.pos++];
    if (c === undefined) return fail("the pattern ends with a backslash");
    if (c === "b") return { t: "assert", kind: "word" };
    if (c === "B") return { t: "assert", kind: "notword" };
    if (c in CLASS_ESCAPES) return { t: "set", test: fast(CLASS_ESCAPES[c]) };
    if (c in CONTROL_ESCAPES) return { t: "set", test: literal(CONTROL_ESCAPES[c]) };
    if (UNSUPPORTED_ESCAPES.has(c)) return fail(/\d/.test(c) ? "back-references aren't supported" : `\\${c} isn't supported`);
    return { t: "set", test: literal(c) };
  }

  /** One member of a `[...]`: a character (with its code, for ranges) or a whole class such as `\d`. */
  private classAtom(): { ch: string } | { test: Test } {
    const c = this.p[this.pos++];
    if (c !== "\\") return { ch: c };
    const e = this.p[this.pos++];
    if (e === undefined) return fail("the pattern ends with a backslash");
    if (e in CLASS_ESCAPES) return { test: CLASS_ESCAPES[e] };
    if (e in CONTROL_ESCAPES) return { ch: CONTROL_ESCAPES[e] };
    if (e === "b") return { ch: "\b" };
    if (UNSUPPORTED_ESCAPES.has(e)) return fail(/\d/.test(e) ? "back-references aren't supported" : `\\${e} isn't supported`);
    return { ch: e };
  }

  private charClass(): Node {
    const negated = this.peek() === "^";
    if (negated) this.pos++;
    const tests: Test[] = [];
    for (;;) {
      const c = this.peek();
      if (c === undefined) return fail("a [ is never closed");
      if (c === "]") {
        this.pos++;
        break;
      }
      const a = this.classAtom();
      if ("test" in a) {
        tests.push(a.test);
        continue;
      }
      if (this.peek() === "-" && this.p[this.pos + 1] !== undefined && this.p[this.pos + 1] !== "]") {
        const save = this.pos;
        this.pos++;
        const b = this.classAtom();
        if ("ch" in b) {
          const lo = a.ch.codePointAt(0)!;
          const hi = b.ch.codePointAt(0)!;
          if (hi < lo) fail(`the range ${a.ch}-${b.ch} is out of order`);
          tests.push((ch) => {
            const x = ch.codePointAt(0)!;
            return x >= lo && x <= hi;
          });
          continue;
        }
        this.pos = save; // `a-\d`: the dash is a character
      }
      tests.push(literal(a.ch));
    }
    const inClass: Test = (ch) => tests.some((t) => t(ch));
    // Case-insensitive, as JavaScript folds it: a character also matches through a case partner of the same length that
    // canonicalises to the same letter. The Kelvin sign (U+212A) and the Angstrom sign (U+212B) fold to `k` and `å` when
    // lower-cased but are their own upper-case letters, so under JavaScript's rule they are not partners of those letters.
    const partners = (ch: string): string[] => {
      const out: string[] = [];
      for (const other of [ch.toLowerCase(), ch.toUpperCase()]) if (other.length === ch.length && other !== ch && canon(other) === canon(ch)) out.push(other);
      return out;
    };
    const matches: Test = (ch) => inClass(ch) || partners(ch).some(inClass);
    return { t: "set", test: fast(negated ? not(matches) : matches) };
  }
}

type Op =
  | { op: "char"; test: Test }
  | { op: "split"; a: number; b: number }
  | { op: "jmp"; to: number }
  | { op: "assert"; kind: "start" | "end" | "word" | "notword" }
  | { op: "match" };

function compile(root: Node): Op[] {
  const prog: Op[] = [];
  let budget = MAX_PROGRAM;
  const push = (o: Op): number => {
    if (prog.length >= MAX_PROGRAM) fail("the pattern is too large to run");
    prog.push(o);
    return prog.length - 1;
  };
  const emit = (n: Node): void => {
    // Charged per node, not per instruction: `(){100}` repeated inside itself emits nothing at all, ten billion times over.
    if (--budget < 0) fail("the pattern is too large to run");
    switch (n.t) {
      case "set":
        push({ op: "char", test: n.test });
        return;
      case "assert":
        push({ op: "assert", kind: n.kind });
        return;
      case "alt": {
        const jumps: number[] = [];
        n.alts.forEach((seq, i) => {
          const last = i === n.alts.length - 1;
          const split = last ? -1 : push({ op: "split", a: 0, b: 0 });
          if (!last) (prog[split] as Extract<Op, { op: "split" }>).a = prog.length;
          for (const s of seq) emit(s);
          if (!last) {
            jumps.push(push({ op: "jmp", to: 0 }));
            (prog[split] as Extract<Op, { op: "split" }>).b = prog.length;
          }
        });
        for (const j of jumps) (prog[j] as Extract<Op, { op: "jmp" }>).to = prog.length;
        return;
      }
      case "rep": {
        for (let i = 0; i < n.min; i++) emit(n.node);
        if (n.max === Infinity) {
          const split = push({ op: "split", a: 0, b: 0 });
          (prog[split] as Extract<Op, { op: "split" }>).a = prog.length;
          emit(n.node);
          push({ op: "jmp", to: split });
          (prog[split] as Extract<Op, { op: "split" }>).b = prog.length;
        } else {
          const splits: number[] = [];
          for (let i = n.min; i < n.max; i++) {
            const split = push({ op: "split", a: 0, b: 0 });
            splits.push(split);
            (prog[split] as Extract<Op, { op: "split" }>).a = prog.length;
            emit(n.node);
          }
          for (const s of splits) (prog[s] as Extract<Op, { op: "split" }>).b = prog.length;
        }
        return;
      }
    }
  };
  emit(root);
  push({ op: "match" });
  return prog;
}

function run(prog: Op[], text: string): boolean {
  const chars = Array.from(text).slice(0, MAX_NAME);
  const n = chars.length;
  const wordAt = (i: number) => i >= 0 && i < n && isWord(chars[i]);
  const seen = new Int32Array(prog.length).fill(-1);
  let current: number[] = [];
  let next: number[] = [];
  const stack: number[] = [];

  /** Adds `pc` and everything reachable from it without reading a character, as of position `pos`; true on a match. */
  const add = (list: number[], pc0: number, pos: number): boolean => {
    stack.push(pc0);
    while (stack.length) {
      const pc = stack.pop()!;
      if (seen[pc] === pos) continue;
      seen[pc] = pos;
      const o = prog[pc];
      switch (o.op) {
        case "match":
          stack.length = 0;
          return true;
        case "jmp":
          stack.push(o.to);
          break;
        case "split":
          stack.push(o.b, o.a);
          break;
        case "assert": {
          const ok =
            o.kind === "start" ? pos === 0 : o.kind === "end" ? pos === n : (wordAt(pos - 1) !== wordAt(pos)) === (o.kind === "word");
          if (ok) stack.push(pc + 1);
          break;
        }
        case "char":
          list.push(pc);
          break;
      }
    }
    return false;
  };

  for (let i = 0; ; i++) {
    // `seen` is stamped with the position, so the start thread and the threads stepped into `i` share one closure.
    if (add(current, 0, i)) return true;
    if (i >= n) return false;
    next = [];
    for (const pc of current) {
      const o = prog[pc];
      if (o.op === "char" && o.test(chars[i]) && add(next, pc + 1, i + 1)) return true;
    }
    current = next;
  }
}

/** Compiles a rule's name pattern, or says in plain words what is wrong with it. */
export function compilePattern(pattern: string): PatternResult {
  if (pattern === "") return { error: "the pattern is empty" };
  if (Array.from(pattern).length > MAX_PATTERN) return { error: `the pattern is longer than ${MAX_PATTERN} characters` };
  try {
    const prog = compile(new Parser(pattern).parse());
    return { test: (name) => run(prog, name) };
  } catch (e) {
    if (e instanceof PatternError) return { error: e.message };
    return { error: "the pattern can't be run" }; // e.g. a stack overflow: a bad rule never breaks the tree
  }
}
