import { useMemo, useState, type ReactNode } from "react";
import type { ColumnDef } from "@tanstack/react-table";
import { Copy, RefreshCw, Settings } from "lucide-react";
import {
  Button, CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog, CommandShortcut,
  CountBadge, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
  EmptyState, EntityGlyph, Field, Hint, IconButton, Input, Kbd, MenuShortcut, Panel, PanelGroup, ProdBadge,
  ResizeHandle, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, SkeletonRows,
  DataGrid, Tabs, TabsList, TabsTrigger, ThemeScope, TooltipProvider, Tree,
  rowHeight, shortcutLabel, type Density, type EntityKind, type ThemeName, type TreeNode,
} from "@/ui";

const SUB_TABS = ["Overview", "Messages", "Dead-letter", "Subscriptions", "Rules", "Properties", "Metrics", "Access", "Settings"];

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b border-line px-4 py-3">
      <h2 className="m-0 mb-2 text-12 font-semibold text-muted">{title}</h2>
      <div className="flex flex-wrap items-start gap-3">{children}</div>
    </section>
  );
}

interface Entity { id: string; label: string; kind?: EntityKind; group?: boolean; prod?: boolean; active?: number; dead?: number; children?: Entity[] }

const NAMESPACES: Entity[] = [
  {
    id: "prod", label: "sb-logistics-prod", kind: "namespace", prod: true, children: [
      {
        id: "prod/queues", label: "Queues", group: true, children: [
          { id: "customs-docs", label: "customs-docs", kind: "queue", active: 112, dead: 0 },
          { id: "invoice-generate", label: "invoice-generate", kind: "queue", active: 1284, dead: 37 },
          { id: "label-print", label: "label-print", kind: "queue", active: 6, dead: 0 },
          { id: "returns-intake", label: "returns-intake", kind: "queue", active: 0, dead: 211 },
        ],
      },
      {
        id: "prod/topics", label: "Topics", group: true, children: [
          {
            id: "carrier-updates", label: "carrier-updates", kind: "topic", children: [
              { id: "audit-log", label: "audit-log", kind: "subscription", active: 0, dead: 0 },
              { id: "eta-recalc", label: "eta-recalc", kind: "subscription", active: 58, dead: 9 },
            ],
          },
          {
            id: "shipment-events", label: "shipment-events", kind: "topic", children: [
              { id: "billing", label: "billing", kind: "subscription", active: 0, dead: 0 },
              { id: "tracking-sync", label: "tracking-sync", kind: "subscription", active: 2906, dead: 3417 },
              { id: "warehouse-notify", label: "warehouse-notify", kind: "subscription", active: 14, dead: 1 },
            ],
          },
          { id: "stock-levels", label: "stock-levels", kind: "topic", children: [{ id: "stock-sync", label: "stock-sync", kind: "subscription", active: 0, dead: 0 }] },
        ],
      },
    ],
  },
  { id: "dev", label: "sb-logistics-dev", kind: "namespace", children: [{ id: "dev/queues", label: "Queues", group: true, children: [] }] },
  { id: "gateway", label: "sb-partner-gateway", kind: "namespace", children: [{ id: "gateway/queues", label: "Queues", group: true, children: [] }] },
];

type EntityNode = TreeNode & { entity: Entity };

function flatten(items: Entity[], open: Set<string>, parentId: string | null = null, depth = 0): EntityNode[] {
  return items.flatMap((entity) => {
    const expandable = !!entity.children?.length || !!entity.group || entity.kind === "namespace";
    const expanded = expandable && open.has(entity.id);
    const self: EntityNode = { id: entity.id, parentId, depth, label: entity.label, expandable, expanded, entity };
    return expanded ? [self, ...flatten(entity.children ?? [], open, entity.id, depth + 1)] : [self];
  });
}

function TreeDemo({ density }: { density: Density }) {
  const [open, setOpen] = useState(
    () => new Set(["prod", "prod/queues", "prod/topics", "carrier-updates", "shipment-events"]),
  );
  const [selectedId, setSelectedId] = useState<string | null>("tracking-sync");
  const nodes = useMemo(() => flatten(NAMESPACES, open), [open]);
  return (
    <div className="h-[360px] w-[260px] border border-line bg-side">
      <Tree
        label="Explorer"
        nodes={nodes}
        selectedId={selectedId}
        onSelect={setSelectedId}
        onActivate={(id) => setOpen((o) => new Set(o).add(id))}
        onToggle={(id, expanded) =>
          setOpen((o) => {
            const next = new Set(o);
            if (expanded) next.add(id);
            else next.delete(id);
            return next;
          })
        }
        rowHeight={rowHeight[density]}
        renderRow={({ entity }) => (
          <>
            {entity.kind && (
              <span className="shrink-0 text-icon"><EntityGlyph kind={entity.kind} /></span>
            )}
            <span className={entity.group ? "truncate font-semibold text-muted" : entity.kind === "namespace" ? "truncate font-semibold" : "truncate"}>
              {entity.label}
            </span>
            {entity.prod && <span className="ml-auto shrink-0 text-11 font-semibold tracking-[0.04em] text-accent">PROD</span>}
            {entity.group && <CountBadge value={entity.children?.length ?? 0} className="ml-auto text-12" />}
            {entity.active != null && (
              <span className="ml-auto flex shrink-0 items-baseline gap-1.5">
                <CountBadge value={entity.active} className="text-12" />
                {!!entity.dead && <CountBadge value={entity.dead} attention className="text-12" />}
              </span>
            )}
          </>
        )}
      />
    </div>
  );
}

interface DeadLetter { seq: string; enqueued: string; subject: string; id: string; reason: string; deliveries: string; size: string }

const DEAD_LETTERS: DeadLetter[] = ([
  ["48213", "15 Oct 09:41:07", "ShipmentDelivered", "7f3c9e21-4a0b", "MaxDeliveryCountExceeded", "10", "1.8 KB"],
  ["48207", "15 Oct 09:40:52", "ShipmentInTransit", "c1d84f0e-99a2", "MaxDeliveryCountExceeded", "10", "2.1 KB"],
  ["48199", "15 Oct 09:40:31", "EtaChanged", "0b6e2a77-1fc4", "Schema validation failed", "1", "0.9 KB"],
  ["48188", "15 Oct 09:39:58", "ShipmentDelivered", "5e02bd13-7c61", "MaxDeliveryCountExceeded", "10", "1.7 KB"],
  ["48176", "15 Oct 09:39:14", "ShipmentException", "a93f0c58-2e7d", "Carrier API returned 503", "4", "3.4 KB"],
  ["48164", "15 Oct 09:38:40", "EtaChanged", "2d7b91c4-ab08", "Schema validation failed", "1", "0.9 KB"],
  ["48151", "15 Oct 09:37:22", "ShipmentInTransit", "e8c13a6f-5d90", "MaxDeliveryCountExceeded", "10", "2.0 KB"],
  ["48140", "15 Oct 09:36:05", "ShipmentDelivered", "61af4e2b-0c37", "TTLExpiredException", "0", "1.8 KB"],
  ["48133", "15 Oct 09:35:47", "ShipmentCreated", "9c5d27e0-8b14", "MaxDeliveryCountExceeded", "10", "4.6 KB"],
  ["48121", "15 Oct 09:34:19", "EtaChanged", "4f80a3d9-62ce", "Schema validation failed", "1", "1.0 KB"],
  ["48117", "15 Oct 09:33:58", "ShipmentException", "b7e6150c-d3a1", "Carrier API returned 503", "4", "3.1 KB"],
  ["48102", "15 Oct 09:32:40", "ShipmentDelivered", "3a2c8f71-e946", "MaxDeliveryCountExceeded", "10", "1.8 KB"],
  ["48096", "15 Oct 09:31:27", "ShipmentInTransit", "d05b6e9a-17f3", "TTLExpiredException", "0", "2.2 KB"],
  ["48085", "15 Oct 09:30:02", "ShipmentCreated", "86e4d1b2-a05c", "MaxDeliveryCountExceeded", "10", "4.4 KB"],
] as const).map(([seq, enqueued, subject, id, reason, deliveries, size]) => ({ seq, enqueued, subject, id, reason, deliveries, size }));

const dim = (v: string) => <span className="text-muted">{v}</span>;

const DEAD_LETTER_COLUMNS: ColumnDef<DeadLetter, string>[] = [
  { accessorKey: "seq", header: "Seq", meta: { mono: true, width: "64px" } },
  { accessorKey: "enqueued", header: "Enqueued", cell: (c) => dim(c.getValue()), meta: { mono: true, width: "118px" } },
  { accessorKey: "subject", header: "Subject", meta: { width: "140px" } },
  { accessorKey: "id", header: "Message ID", cell: (c) => dim(c.getValue()), meta: { mono: true, width: "120px" } },
  { accessorKey: "reason", header: "Dead-letter reason", meta: { width: "minmax(190px, 1fr)" } },
  { accessorKey: "deliveries", header: "Del.", meta: { align: "end", width: "44px" } },
  { accessorKey: "size", header: "Size", cell: (c) => dim(c.getValue()), meta: { mono: true, align: "end", width: "60px" } },
];

function GridDemo({ density }: { density: Density }) {
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set(["48199"]));
  const [focusId, setFocusId] = useState<string | null>("48199");
  const common = { columns: DEAD_LETTER_COLUMNS, getRowId: (r: DeadLetter) => r.seq, rowHeight: rowHeight[density] };
  return (
    <div className="flex w-full flex-col gap-3">
      <div className="h-[360px] w-full border border-line">
        <DataGrid
          {...common}
          label="Dead-lettered messages"
          rows={DEAD_LETTERS}
          selectedIds={selected}
          focusId={focusId}
          onSelectionChange={(ids, f) => { setSelected(ids); setFocusId(f); }}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="h-[130px] min-w-0 border border-line">
          <DataGrid {...common} label="Loading messages" rows={[]} loading selectedIds={new Set()} focusId={null} onSelectionChange={() => {}} />
        </div>
        <div className="h-[130px] min-w-0 border border-line">
          <DataGrid
            {...common}
            label="No dead-lettered messages"
            rows={[]}
            empty={<EmptyState title="No dead letters" body="Messages that fail delivery land here." />}
            selectedIds={new Set()}
            focusId={null}
            onSelectionChange={() => {}}
          />
        </div>
      </div>
    </div>
  );
}

function Overlays() {
  const [sheetOpen, setSheetOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  return (
    <Section title="Overlays">
      <div className="w-full">
        <Tabs defaultValue="orders">
          <TabsList variant="strip" aria-label="Open entities">
            <TabsTrigger value="orders"><EntityGlyph kind="queue" />orders</TabsTrigger>
            <TabsTrigger value="shipment-events"><EntityGlyph kind="topic" />shipment-events</TabsTrigger>
            <TabsTrigger value="audit"><EntityGlyph kind="subscription" />audit</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>
      <div className="w-full border-b border-line">
        <Tabs defaultValue="Overview">
          <TabsList variant="underline" aria-label="Views">
            {SUB_TABS.map((t) => <TabsTrigger key={t} value={t}>{t}</TabsTrigger>)}
          </TabsList>
        </Tabs>
      </div>
      <DropdownMenu>
        <DropdownMenuTrigger asChild><Button>Logistics</Button></DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem><Copy />Copy name<MenuShortcut>{shortcutLabel("mod+c")}</MenuShortcut></DropdownMenuItem>
          <DropdownMenuItem><RefreshCw />Refresh<MenuShortcut>{shortcutLabel("f5")}</MenuShortcut></DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem><Settings />Manage profiles</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Hint label="Refresh" shortcut="f5"><IconButton label="Refresh entity"><RefreshCw /></IconButton></Hint>
      <Button onClick={() => setSheetOpen(true)}>Open sheet</Button>
      <Button onClick={() => setPaletteOpen(true)}>Open palette <Kbd shortcut="mod+k" /></Button>
      <Sheet
        open={sheetOpen}
        onOpenChange={setSheetOpen}
        title="Add connection"
        description="Saved on this computer."
        footer={<><Button variant="quiet" onClick={() => setSheetOpen(false)}>Cancel</Button><Button variant="primary">Save profile</Button></>}
      >
        <div className="flex flex-col gap-3">
          <Field label="Name">{(ids) => <Input {...ids} placeholder="Logistics" />}</Field>
          <Field label="Connection string" hint="Stays in your keychain">{(ids) => <Input {...ids} mono placeholder="Endpoint=sb://…" />}</Field>
        </div>
      </Sheet>
      <CommandPaletteDialog open={paletteOpen} onOpenChange={setPaletteOpen}>
        <CommandInput placeholder="Type a command or search entities" />
        <CommandList>
          <CommandGroup heading="App">
            <CommandItem><RefreshCw />Refresh<CommandShortcut><Kbd shortcut="f5" /></CommandShortcut></CommandItem>
            <CommandItem><Settings />Toggle theme</CommandItem>
          </CommandGroup>
          <CommandGroup heading="Entities">
            <CommandItem><EntityGlyph kind="queue" />orders</CommandItem>
            <CommandItem><EntityGlyph kind="topic" />shipment-events</CommandItem>
          </CommandGroup>
        </CommandList>
      </CommandPaletteDialog>
      <div className="h-[400px] w-full border border-line">
        <PanelGroup direction="horizontal">
          <Panel defaultSize={35} minSize={20}><div className="p-3 text-muted">Tree</div></Panel>
          <ResizeHandle />
          <Panel minSize={30}><div className="p-3 text-muted">Content</div></Panel>
        </PanelGroup>
      </div>
    </Section>
  );
}

function Column({ theme, density }: { theme: ThemeName; density: Density }) {
  return (
    <ThemeScope theme={theme} density={density}>
      <div>
        <Section title="Buttons">
          <Button>Default</Button>
          <Button variant="quiet">Quiet</Button>
          <Button variant="primary">Save profile</Button>
          <Button variant="danger">Delete profile</Button>
          <Button disabled>Disabled</Button>
          <IconButton label="Refresh"><RefreshCw /></IconButton>
        </Section>
        <Section title="Inputs">
          <div className="w-64">
            <Field label="Name" hint="Shown in the connection list">{(ids) => <Input {...ids} placeholder="Logistics" />}</Field>
          </div>
          <div className="w-64">
            <Field label="Connection string" error="This is not a Service Bus connection string.">
              {(ids) => <Input {...ids} mono defaultValue="Endpoint=sb://…" />}
            </Field>
          </div>
          <div className="w-48">
            <Select defaultValue="amqp">
              <SelectTrigger aria-label="Transport"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="amqp">AMQP</SelectItem>
                <SelectItem value="websockets">AMQP over WebSockets</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </Section>
        <Section title="Badges and counts">
          <ProdBadge />
          <CountBadge value={3417} attention />
          <CountBadge value={0} attention />
          <CountBadge value={2906} />
          <CountBadge value={null} />
          <CountBadge value={120} quality={{ approximate: true }} />
          <Kbd shortcut="mod+k" />
          <Kbd shortcut="f5" />
        </Section>
        <Section title="Glyphs">
          {(["namespace", "queue", "topic", "subscription", "rule"] as const).map((k) => (
            <span key={k} className="flex items-center gap-1.5 text-icon"><EntityGlyph kind={k} /><span className="text-fg">{k}</span></span>
          ))}
        </Section>
        <Section title="Tree">
          <TreeDemo density={density} />
        </Section>
        <Section title="Grid">
          <GridDemo density={density} />
        </Section>
        <Section title="Loading and empty">
          <div className="w-72 border border-line"><SkeletonRows rows={4} columns={2} /></div>
          <EmptyState title="No dead letters" body="Messages that fail delivery land here. This subscription has none." />
          <EmptyState tone="error" title="Can't list subscriptions" body="This identity needs Listen or Manage on shipment-events." action={<Button>Retry</Button>} />
        </Section>
        <Overlays />
      </div>
    </ThemeScope>
  );
}

export default function Catalogue() {
  const [density, setDensity] = useState<Density>("compact");
  return (
    <TooltipProvider>
      <div data-shot-scroller className="h-full overflow-auto">
        <div className="flex items-center gap-2 border-b border-line bg-chrome px-4 py-2">
          <span className="font-semibold">Component catalogue</span>
          <Button variant="quiet" onClick={() => setDensity(density === "compact" ? "comfortable" : "compact")}>
            Density: {density}
          </Button>
        </div>
        <div className="grid grid-cols-2">
          <Column theme="dark" density={density} />
          <Column theme="light" density={density} />
        </div>
      </div>
    </TooltipProvider>
  );
}
