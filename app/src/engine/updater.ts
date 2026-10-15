import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { inTauri } from "./ipc";

export interface UpdateInfo { version: string; notes: string | null; date: string | null }
export interface Updater {
  check(): Promise<UpdateInfo | null>;
  /** `onProgress` reports the download percent (null while the size is unknown); `onInstalling` fires when the
   *  download is done and the update is being applied. Resolves just before the app restarts. */
  install(onProgress: (pct: number | null) => void, onInstalling?: () => void): Promise<void>;
}

let pending: Update | null = null;

const tauriUpdater: Updater = {
  async check() {
    if (!inTauri()) return null;
    pending = await check();
    return pending ? { version: pending.version, notes: pending.body ?? null, date: pending.date ?? null } : null;
  },
  async install(onProgress, onInstalling) {
    if (!pending) throw new Error("No update is ready to install. Check for updates again.");
    let total = 0;
    let got = 0;
    await pending.downloadAndInstall((e) => {
      if (e.event === "Started") total = e.data.contentLength ?? 0;
      else if (e.event === "Progress") {
        got += e.data.chunkLength;
        onProgress(total ? Math.round((got / total) * 100) : null);
      } else if (e.event === "Finished") onInstalling?.();
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
export const installUpdate = (onProgress: (pct: number | null) => void, onInstalling?: () => void) =>
  impl.install(onProgress, onInstalling);
