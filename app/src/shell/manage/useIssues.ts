import { useEffect, useState } from "react";
import type { FieldIssue } from "@/engine";

/**
 * The engine's problems with a form as it is edited: checked `ms` after the last change. `pending` is true while a
 * check for the current value has not come back, so a primary button can wait for it.
 */
export function useIssues<T>(value: T, check: (v: T) => Promise<FieldIssue[]>, enabled = true, ms = 250): { issues: FieldIssue[]; pending: boolean } {
  const key = JSON.stringify(value);
  const [state, setState] = useState<{ key: string; issues: FieldIssue[] } | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    const t = setTimeout(() => {
      check(JSON.parse(key) as T)
        .then((issues) => alive && setState({ key, issues }))
        .catch(() => alive && setState({ key, issues: [] }));
    }, ms);
    return () => {
      alive = false;
      clearTimeout(t);
    };
    // `check` is a stable engine call; the value is compared by its JSON.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, enabled, ms]);
  if (!enabled) return { issues: [], pending: false };
  return { issues: state?.key === key ? state.issues : (state?.issues ?? []), pending: state?.key !== key };
}

/** The first problem with `field`, if any. */
export const issueOf = (issues: FieldIssue[], field: string) => issues.find((i) => i.field === field)?.message;
