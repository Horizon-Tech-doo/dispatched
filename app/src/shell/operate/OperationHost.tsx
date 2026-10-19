import type { OperationSeed } from "./seed";

/** Renders the sheet of one operation. */
export function OperationHost({ seed }: { seed: OperationSeed }) {
  switch (seed.op) {
    default:
      return null;
  }
}
