import { Check, ChevronDown } from "lucide-react";
import { useProfiles } from "@/engine";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger, ProdBadge,
} from "@/ui";
import { useShell } from "./context";

export const shortHost = (host: string) => host.replace(/\.servicebus\.(windows\.net|chinacloudapi\.cn|usgovcloudapi\.net)$/, "");

export function ConnectionSelector() {
  const { conn, openSheet } = useShell();
  const profiles = useProfiles().data ?? [];
  const current = profiles.find((p) => p.id === conn.profileId);
  const label =
    conn.status === "connecting" && current ? `Connecting to ${current.name}…`
    : conn.status === "connected" && current ? current.name
    : "Choose a connection";
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={`Connection: ${label}`}
          className="flex h-[26px] max-w-[420px] items-center gap-2 rounded-ctl border border-line bg-field px-2 text-12 hover:bg-hover"
        >
          <span className="truncate font-semibold">{label}</span>
          {conn.status === "connected" && current && <span className="truncate text-muted">{shortHost(current.host)}</span>}
          {conn.status === "connected" && current?.production && <ProdBadge />}
          <ChevronDown aria-hidden="true" className="size-3 shrink-0 text-icon" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="min-w-[300px]">
        <DropdownMenuLabel>Connections</DropdownMenuLabel>
        {profiles.map((p) => (
          <DropdownMenuItem key={p.id} onSelect={() => void conn.connect(p.id)}>
            <span className="w-3.5">{p.id === conn.profileId && conn.status === "connected" && <Check />}</span>
            <span className="truncate">{p.name}</span>
            <span className="truncate text-faint">{shortHost(p.host)}</span>
            {p.production && <ProdBadge className="ml-auto" />}
          </DropdownMenuItem>
        ))}
        {profiles.length > 0 && <DropdownMenuSeparator />}
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profile", profileId: null })}>Add connection…</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profiles" })}>Manage connections…</DropdownMenuItem>
        {conn.status === "connected" && <DropdownMenuItem onSelect={() => void conn.disconnect()}>Disconnect</DropdownMenuItem>}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
