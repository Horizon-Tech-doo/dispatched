import { useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { checkForUpdate, useAppInfo } from "@/engine";

const START_DELAY_MS = 3000;

/** The shared update check. By default it runs once, 3 s after mount; with `onDemand` it only reads the shared
 *  result and `refetch()` checks again. A build that doesn't update itself (Mac App Store) never checks. */
export function useUpdate({ onDemand = false }: { onDemand?: boolean } = {}) {
  const updates = useAppInfo().data?.updates === true;
  const [armed, setArmed] = useState(false);
  useEffect(() => {
    if (onDemand) return;
    const t = setTimeout(() => setArmed(true), START_DELAY_MS);
    return () => clearTimeout(t);
  }, [onDemand]);
  return useQuery({ queryKey: ["update"], queryFn: checkForUpdate, enabled: armed && updates, staleTime: Infinity, retry: false });
}
