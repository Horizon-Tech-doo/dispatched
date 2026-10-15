import { useIsFetching, useQueryClient } from "@tanstack/react-query";
import { useSyncExternalStore } from "react";
import type { EngineError } from "@/engine";

export { useIsFetching, useQueryClient };

/** Observe a query's cached state (data time, error) without fetching it. */
export function useQueryState(queryKey: readonly unknown[]): { dataUpdatedAt: number; error: EngineError | null } {
  const qc = useQueryClient();
  const cache = qc.getQueryCache();
  const read = () => {
    const q = cache.find({ queryKey, exact: true });
    return `${q?.state.dataUpdatedAt ?? 0}|${q?.state.error ? (q.state.error as EngineError).kind : ""}`;
  };
  const snap = useSyncExternalStore((cb) => cache.subscribe(cb), read);
  const q = cache.find({ queryKey, exact: true });
  void snap;
  return { dataUpdatedAt: q?.state.dataUpdatedAt ?? 0, error: (q?.state.error as EngineError | null) ?? null };
}
