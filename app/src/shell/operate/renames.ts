/** Renames waiting for their job to finish, by job id: the tab of the old name then follows the new one. */
const pending = new Map<string, { from: string; to: string }>();

/** A rename sheet started job `jobId`, renaming tab id `from` to `to`. */
export const noteRename = (jobId: string, from: string, to: string) => void pending.set(jobId, { from, to });

/** The rename job `jobId` made, once. */
export function takeRename(jobId: string): { from: string; to: string } | null {
  const r = pending.get(jobId) ?? null;
  pending.delete(jobId);
  return r;
}
