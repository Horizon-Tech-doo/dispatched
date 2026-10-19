const TEXT = {
  empty: "Empty body (0 bytes)",
  binary: "Not valid UTF-8, shown as hex",
  "too-large": "Too large to format; showing raw text.",
} as const;

/** One muted line that says why a body is not shown as formatted text. The shell places it above (or instead of) the view. */
export function BodyState({ kind }: { kind: keyof typeof TEXT }) {
  return <p className="m-0 text-12 text-muted">{TEXT[kind]}</p>;
}
