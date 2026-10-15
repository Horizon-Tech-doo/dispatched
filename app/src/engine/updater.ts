import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { inTauri } from "./ipc";

export interface UpdateInfo { version: string; notes: string | null; date: string | null }
export interface Updater {
  check(): Promise<UpdateInfo | null>;
  install(onProgress: (pct: number | null) => void): Promise<void>;
}

let pending: Update | null = null;

const tauriUpdater: Updater = {
  async check() {
    if (!inTauri()) return null;
    pending = await check();
    return pending ? { version: pending.version, notes: pending.body ?? null, date: pending.date ?? null } : null;
  },
  async install(onProgress) {
    if (!pending) return;
    let total = 0;
    let got = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      else if (e.event === "Progress") {
        got += e.data.chunkLength;
        onProgress(total ? Math.round((got / total) * 100) : null);
      }
    });
    await relaunch();
  },
};

let impl: Updater = tauriUpdater;

/** Swap the updater (tests and mock builds). */
export function setUpdater(u: Updater): void {
  impl = u;
}
/** Back to the real updater (test cleanup). */
export function resetUpdater(): void {
  impl = tauriUpdater;
}
export const checkForUpdate = () => impl.check();
export const installUpdate = (onProgress: (pct: number | null) => void) => impl.install(onProgress);
