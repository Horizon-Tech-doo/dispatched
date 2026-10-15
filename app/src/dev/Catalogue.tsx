import { useState, type ReactNode } from "react";
import { RefreshCw } from "lucide-react";
import {
  Button, CountBadge, EmptyState, EntityGlyph, Field, IconButton, Input, Kbd, ProdBadge,
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue, SkeletonRows, ThemeScope,
  type Density, type ThemeName,
} from "@/ui";

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="border-b border-line px-4 py-3">
      <h2 className="m-0 mb-2 text-12 font-semibold text-muted">{title}</h2>
      <div className="flex flex-wrap items-start gap-3">{children}</div>
    </section>
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
      </div>
    </ThemeScope>
  );
}

export default function Catalogue() {
  const [density, setDensity] = useState<Density>("compact");
  return (
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
  );
}
