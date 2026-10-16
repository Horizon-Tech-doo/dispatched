import { HEX_ROW, hexRow } from "./format";
import { VirtualLines } from "./VirtualLines";

/** Any bytes as `offset  hex  |text|` rows of 16, virtualised. */
export function HexView({ bytes, label }: { bytes: Uint8Array; label: string }) {
  return (
    <VirtualLines
      label={label}
      resetKey={bytes}
      count={Math.ceil(bytes.length / HEX_ROW)}
      renderRow={(i) => {
        const r = hexRow(bytes, i * HEX_ROW);
        return (
          <>
            <span className="px-3 text-muted">{r.offset}</span>
            <span className="pr-3 text-fg">{r.hex}</span>
            <span className="pr-4 text-muted">{r.ascii}</span>
          </>
        );
      }}
    />
  );
}
