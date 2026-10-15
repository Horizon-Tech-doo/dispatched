import type { ReactNode } from "react";
import { useAppInfo } from "@/engine";
import { Button, Segmented, Sheet } from "@/ui";
import { useShell } from "../context";
import type { AutoRefresh } from "../state";

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 py-3 first:pt-0 last:pb-0">
      <div className="flex min-w-0 flex-col">
        <span className="text-12 font-semibold">{label}</span>
        {hint && <span className="text-11 text-muted">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

export function SettingsSheet() {
  const { prefs, setPrefs, closeSheet, openSheet } = useShell();
  const info = useAppInfo().data;
  return (
    <Sheet open onOpenChange={(o) => !o && closeSheet()} title="Settings">
      <div className="flex flex-col divide-y divide-line">
        <Row label="Theme" hint="System follows the operating system.">
          <Segmented
            label="Theme"
            value={prefs.theme}
            onChange={(theme) => setPrefs({ theme })}
            options={[{ value: "system", label: "System" }, { value: "dark", label: "Dark" }, { value: "light", label: "Light" }]}
          />
        </Row>
        <Row label="Density" hint="Row height in the tree and grids.">
          <Segmented
            label="Density"
            value={prefs.density}
            onChange={(density) => setPrefs({ density })}
            options={[{ value: "compact", label: "Compact" }, { value: "comfortable", label: "Comfortable" }]}
          />
        </Row>
        <Row label="Auto-refresh" hint="Refreshes the connected namespace's counts.">
          <Segmented
            label="Auto-refresh"
            value={String(prefs.autoRefresh)}
            onChange={(v) => setPrefs({ autoRefresh: Number(v) as AutoRefresh })}
            options={[{ value: "0", label: "Off" }, { value: "30", label: "30 s" }, { value: "60", label: "60 s" }]}
          />
        </Row>
        <Row label="Version">
          <div className="flex items-center gap-3">
            <span className="text-12 tabular-nums text-muted">{info?.version}</span>
            <Button size="sm" onClick={() => openSheet({ kind: "update" })}>Check for updates</Button>
          </div>
        </Row>
      </div>
    </Sheet>
  );
}
