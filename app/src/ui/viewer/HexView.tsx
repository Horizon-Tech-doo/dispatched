import { Fragment } from "react";
import { HEX_ROW, hexRow } from "./format";
import { NoticeRow, VirtualLines } from "./VirtualLines";

const COLUMNS = Array.from({ length: HEX_ROW }, (_, i) => i.toString(16).padStart(2, "0"));
const HEX_HEADER = `${COLUMNS.slice(0, 8).join(" ")}  ${COLUMNS.slice(8).join(" ")}`.padEnd(HEX_ROW * 3, " ");

/** Zero bytes are dimmed so runs of padding recede. A byte is exactly two hex digits, so "00" only ever occurs as a whole byte. */
function HexBytes({ hex }: { hex: string }) {
  return (
    <>
      {hex.split(/(00)/).map((part, i) => (
        <Fragment key={i}>{part === "00" ? <span className="text-faint">00</span> : part}</Fragment>
      ))}
    </>
  );
}

/** Any bytes as `offset  hex  |text|` rows of 16, virtualised, under a sticky column header. `notice` is shown as a last row. */
export function HexView({ bytes, label, notice }: { bytes: Uint8Array; label: string; notice?: string }) {
  const rows = Math.ceil(bytes.length / HEX_ROW);
  return (
    <VirtualLines
      label={label}
      resetKey={bytes}
      count={rows + (notice ? 1 : 0)}
      header={
        <>
          <span className="px-3 text-muted">{"Offset".padEnd(8, " ")}</span>
          <span className="pr-3 text-muted">{HEX_HEADER}</span>
          <span className="pr-4 text-muted">Text</span>
        </>
      }
      renderRow={(i) => {
        if (i >= rows) return <NoticeRow>{notice}</NoticeRow>;
        const r = hexRow(bytes, i * HEX_ROW);
        return (
          <>
            <span className="px-3 text-muted">{r.offset}</span>
            <span className="pr-3 text-fg"><HexBytes hex={r.hex} /></span>
            <span className="pr-4 text-muted">{r.ascii}</span>
          </>
        );
      }}
    />
  );
}
