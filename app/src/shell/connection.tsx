import { useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { commands, type ConnectionView, type EngineError } from "@/engine";

export type ConnStatus = "idle" | "connecting" | "connected" | "failed";

interface ConnState {
  status: ConnStatus;
  view: ConnectionView | null;
  error: EngineError | null;
  profileId: string | null;
}

const idle: ConnState = { status: "idle", view: null, error: null, profileId: null };

export function useConnectionController() {
  const qc = useQueryClient();
  const [s, setS] = useState<ConnState>(idle);
  const current = useRef(s);
  current.current = s;
  const autoTried = useRef(false);
  // Every connect or disconnect takes a number; a connect whose number is no longer the latest
  // finished after the user moved on, and its result is dropped.
  const latest = useRef(0);

  const connect = useCallback(
    async (profileId: string) => {
      const n = ++latest.current;
      setS({ status: "connecting", view: null, error: null, profileId });
      try {
        const view = await commands.connect(profileId);
        if (n !== latest.current) return;
        qc.removeQueries({ queryKey: ["entities"] });
        qc.removeQueries({ queryKey: ["subscriptions"] });
        setS({ status: "connected", view, error: null, profileId });
        void commands.settingsSet("last_profile", profileId).catch(() => {});
      } catch (e) {
        if (n !== latest.current) return;
        setS({ status: "failed", view: null, error: e as EngineError, profileId });
      }
    },
    [qc],
  );

  const disconnect = useCallback(async () => {
    latest.current++;
    await commands.disconnect().catch(() => {});
    qc.removeQueries({ queryKey: ["entities"] });
    qc.removeQueries({ queryKey: ["subscriptions"] });
    setS(idle);
    void commands.settingsSet("last_profile", null).catch(() => {});
  }, [qc]);

  /** The engine already closed this connection (its sign-in went away): drop the shell's state for it without
   *  asking the engine to disconnect again. A no-op unless that connection is the one the shell shows. */
  const closedByEngine = useCallback(
    (profileId: string) => {
      if (current.current.profileId !== profileId || current.current.status !== "connected") return;
      latest.current++;
      qc.removeQueries({ queryKey: ["entities"] });
      qc.removeQueries({ queryKey: ["subscriptions"] });
      setS(idle);
      void commands.settingsSet("last_profile", null).catch(() => {});
    },
    [qc],
  );

  const retry = useCallback(() => {
    if (s.profileId) void connect(s.profileId);
  }, [connect, s.profileId]);

  // Reopen the last connection once on start (StrictMode runs effects twice).
  useEffect(() => {
    if (autoTried.current) return;
    autoTried.current = true;
    void Promise.all([commands.settingsGet("last_profile"), commands.profilesList()])
      .then(([last, profiles]) => {
        if (typeof last === "string" && profiles.some((p) => p.id === last)) void connect(last);
      })
      .catch(() => {});
  }, [connect]);

  return { ...s, connect, disconnect, closedByEngine, retry };
}

export type ConnectionController = ReturnType<typeof useConnectionController>;
