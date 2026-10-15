import type { ReactNode, SVGProps } from "react";

export type EntityKind = "namespace" | "queue" | "topic" | "subscription" | "rule";

type GlyphProps = Omit<SVGProps<SVGSVGElement>, "children"> & { size?: number };

function Glyph({ size = 14, children, ...rest }: GlyphProps & { children: ReactNode }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth={1.3}
      strokeLinejoin="round" strokeLinecap="round" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

export const NamespaceGlyph = (p: GlyphProps) => (
  <Glyph {...p}><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M2 6.2h12" /></Glyph>
);
export const QueueGlyph = (p: GlyphProps) => (
  <Glyph {...p}><rect x="1.5" y="4.5" width="13" height="7" rx="1" /><path d="M5.8 4.5v7M10.2 4.5v7" /></Glyph>
);
export const TopicGlyph = (p: GlyphProps) => (
  <Glyph {...p}><circle cx="3.8" cy="8" r="1.8" /><path d="M5.6 8h3.2M8.8 8l4.4-4M8.8 8h4.4M8.8 8l4.4 4" /></Glyph>
);
export const SubscriptionGlyph = (p: GlyphProps) => (
  <Glyph {...p}><path d="M2 9.5 3.8 4h8.4L14 9.5V12H2z" /><path d="M2 9.5h3.2l.9 1.4h3.8l.9-1.4H14" /></Glyph>
);
export const RuleGlyph = (p: GlyphProps) => (
  <Glyph {...p}><path d="M2.5 3.5h11l-4.2 5v4l-2.6 1.2V8.5z" /></Glyph>
);

const byKind = { namespace: NamespaceGlyph, queue: QueueGlyph, topic: TopicGlyph, subscription: SubscriptionGlyph, rule: RuleGlyph };

export function EntityGlyph({ kind, ...rest }: GlyphProps & { kind: EntityKind }) {
  const G = byKind[kind];
  return <G {...rest} />;
}
