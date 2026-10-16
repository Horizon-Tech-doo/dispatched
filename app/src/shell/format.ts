/** `812 B`, `1.2 KiB`, `4.0 MiB`, `1.1 GiB`.
 *  Belongs in `ui/lib/format.ts` (next to `formatNumber`); kept in the shell while `ui/` is being edited by another task. */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let v = n / 1024;
  let u = 0;
  while (v >= 1024 && u < units.length - 1) {
    v /= 1024;
    u++;
  }
  return `${v.toFixed(1)} ${units[u]}`;
}
