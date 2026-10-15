import { useSyncExternalStore } from "react";
import { installUpdate } from "@/engine";

export type InstallPhase = "idle" | "downloading" | "installing" | "restarting";
export interface InstallState {
  phase: InstallPhase;
  /** Download percent; null while the size is unknown. */
  progress: number | null;
  error: string | null;
}

const idle: InstallState = { phase: "idle", progress: null, error: null };
let state = idle;
const subs = new Set<() => void>();
const set = (next: InstallState) => {
  state = next;
  subs.forEach((f) => f());
};

/** What went wrong, as text. The updater plugin rejects with plain strings, not Error objects. */
export function reasonOf(e: unknown): string {
  return (e as { message?: string } | null)?.message ?? String(e);
}

/** The install lives outside the sheet so its progress survives the sheet being re-rendered or swapped. */
export const installStore = {
  get: () => state,
  subscribe: (cb: () => void) => {
    subs.add(cb);
    return () => subs.delete(cb);
  },
  /** An install is under way; the update sheet cannot be closed or replaced meanwhile. */
  busy: () => state.phase !== "idle",
  reset: () => set(idle),
  async start() {
    if (state.phase !== "idle") return;
    set({ phase: "downloading", progress: null, error: null });
    try {
      await installUpdate(
        (progress) => set({ ...state, phase: "downloading", progress }),
        () => set({ ...state, phase: "installing" }),
      );
      set({ ...state, phase: "restarting" });
    } catch (e) {
      set({ phase: "idle", progress: null, error: reasonOf(e) });
    }
  },
};

export const useInstall = () => useSyncExternalStore(installStore.subscribe, installStore.get);
