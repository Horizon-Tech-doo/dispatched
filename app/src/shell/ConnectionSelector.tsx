import { Check, ChevronDown, CircleAlert } from "lucide-react";
import { useProfiles } from "@/engine";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger, LabelSwatch, ProdBadge,
} from "@/ui";
import { useShell } from "./context";
import { useConnectionTrouble } from "./trouble";

export const shortHost = (host: string) => host.replace(/\.servicebus\.(windows\.net|chinacloudapi\.cn|usgovcloudapi\.net)$/, "");

export function ConnectionSelector() {
  const { conn, openSheet } = useShell();
  const profiles = useProfiles().data ?? [];
  const current = profiles.find((p) => p.id === conn.profileId);
  const trouble = useConnectionTrouble();
  const troubleText = trouble === "lost" ? ", connection lost" : trouble === "sign-in" ? ", sign-in expired" : trouble ? ", credentials rejected" : "";
  const label =
    conn.status === "connecting" && current ? `Connecting to ${current.name}…`
    : conn.status === "connected" && current ? current.name
    : "Choose a connection";
  const description =
    conn.status === "connected" && current
      ? `${current.name}, ${shortHost(current.host)}${current.production ? ", production" : ""}${troubleText}`
      : label;
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          aria-label={`Connection: ${description}`}
          className="flex h-[26px] max-w-[420px] items-center gap-2 rounded-ctl border border-line bg-field px-2 text-12 hover:bg-hover"
        >
          {conn.status === "connected" && current && <LabelSwatch color={current.color} />}
          <span className="truncate font-semibold">{label}</span>
          {conn.status === "connected" && current && <span className="truncate text-muted">{shortHost(current.host)}</span>}
          {conn.status === "connected" && current?.production && <ProdBadge />}
          {trouble && <CircleAlert aria-hidden="true" className="size-3.5 shrink-0 text-accent" />}
          <ChevronDown aria-hidden="true" className="size-3 shrink-0 text-icon" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent className="min-w-[300px]">
        <DropdownMenuLabel>Connections</DropdownMenuLabel>
        {profiles.map((p) => (
          <DropdownMenuItem key={p.id} onSelect={() => void conn.connect(p.id)}>
            <span aria-hidden="true" className="w-3.5">{p.id === conn.profileId && conn.status === "connected" && <Check />}</span>
            <LabelSwatch color={p.color} />
            <span className="truncate">{p.name}</span>
            <span className="truncate text-faint">{shortHost(p.host)}</span>
            {p.production && <ProdBadge className="ml-auto" />}
          </DropdownMenuItem>
        ))}
        {profiles.length > 0 && <DropdownMenuSeparator />}
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profile", profileId: null })}>
          <span aria-hidden="true" className="w-3.5" />
          Add connection…
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openSheet({ kind: "profiles" })}>
          <span aria-hidden="true" className="w-3.5" />
          Manage connections…
        </DropdownMenuItem>
        {conn.status === "connected" && <DropdownMenuItem onSelect={() => void conn.disconnect()}>
            <span aria-hidden="true" className="w-3.5" />
            Disconnect
          </DropdownMenuItem>}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
