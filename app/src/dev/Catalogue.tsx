import { useState, type ReactNode } from "react";
import { Copy, RefreshCw, Settings } from "lucide-react";
import {
  Button, CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog, CommandShortcut,
  CountBadge, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
  EmptyState, EntityGlyph, Field, Hint, IconButton, Input, Kbd, MenuShortcut, Panel, PanelGroup, ProdBadge,
  ResizeHandle, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sheet, SkeletonRows,
  Tabs, TabsList, TabsTrigger, ThemeScope, TooltipProvider,
  shortcutLabel, type Density, type ThemeName,
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
