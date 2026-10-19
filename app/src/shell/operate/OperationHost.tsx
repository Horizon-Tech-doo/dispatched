import { ComposerSheet } from "./ComposerSheet";
import { ImportSheet } from "./ImportSheet";
import type { OperationSeed } from "./seed";

/** Renders the sheet of one operation. */
export function OperationHost({ seed }: { seed: OperationSeed }) {
  switch (seed.op) {
    case "send":
      return <ComposerSheet entity={seed.entity} clone={seed.clone} />;
    case "import":
      return <ImportSheet entity={seed.entity} />;
    default:
      return null;
  }
}

/** A key per opened operation: another seed starts a fresh sheet. A cloned message counts by its sequence number (its
 *  body can be megabytes). */
export const operationKey = (seed: OperationSeed): string =>
  JSON.stringify(seed, (k, v) => ((k === "clone" || k === "detail") && v ? `seq:${(v as { sequence_number: number }).sequence_number}` : v));
