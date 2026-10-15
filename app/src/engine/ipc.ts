import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Unlisten = () => void;

export interface IpcTransport {
  invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  listen<T>(event: string, cb: (payload: T) => void): Promise<Unlisten>;
}

export class EngineError extends Error {
  readonly kind: string;
  constructor(kind: string, message: string) {
    super(message);
    this.name = "EngineError";
    this.kind = kind;
  }
}

const tauri: IpcTransport = {
  invoke: (cmd, args) => invoke(cmd, args),
  listen: (event, cb) => listen(event, (e) => cb(e.payload as never)),
};

let transport: IpcTransport = tauri;

/** Swap the transport (tests and mock builds). */
export function setTransport(t: IpcTransport): void {
  transport = t;
}

/** True inside the desktop app (false in the browser mock and tests). */
export const inTauri = (): boolean => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export function toEngineError(e: unknown): EngineError {
  if (e instanceof EngineError) return e;
  if (e && typeof e === "object" && "kind" in e && "message" in e) {
    return new EngineError(String((e as { kind: unknown }).kind), String((e as { message: unknown }).message));
  }
  return new EngineError("unknown", e instanceof Error ? e.message : String(e));
}

export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await transport.invoke<T>(cmd, args);
  } catch (e) {
    throw toEngineError(e);
  }
}

export function subscribe<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  return transport.listen<T>(event, cb);
}
