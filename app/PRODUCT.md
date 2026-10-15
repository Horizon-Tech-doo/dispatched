# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack
Existing codebase: Tauri desktop app (React 19, TypeScript, Vite, Tailwind 4, shadcn/ui) for macOS, Windows and Linux. The UI runs in the Tauri webview, so the platform is `web`; there are no native iOS or Android surfaces.

## Users
Developers and on-call operators who own Azure Service Bus integrations. They open the app during incidents ("why are 34,000 messages in this DLQ?") and in daily work (checking a subscription, finding one message). They know Service Bus well and use IDEs and database clients all day.

## Product Purpose
Dispatched (desktop) is a cross-platform desktop explorer for Azure Service Bus: browse namespaces, inspect queues, topics and subscriptions, and repair dead-letter queues. It replaces the Windows-only ServiceBusExplorer, which is the feature reference only, not the design reference.

Success: the user opens it against a production namespace and immediately sees where attention is needed: which entities have dead letters, which profile is production, what failed. It feels like a precise professional tool, never a generated template.

## Positioning
A cross-platform, keyboard-first workbench for Service Bus, where the Windows-only ServiceBusExplorer is the existing alternative. The mode is Operate: a tool people are inside of for hours, where density, keyboard speed and trust beat discovery and delight.

## Operating Context
Used against live Azure namespaces, often production, often under incident pressure, alongside IDEs and database clients. Works offline for the app itself; no network fonts.

## Capabilities and Constraints
- Desktop app (Tauri): macOS, Windows, Linux. Works offline; no network fonts.
- Dark and light themes, following the OS by default.
- Production profiles are always visibly marked.
- Nothing destructive happens without a confirmation in context (no modal dialogs).
- Secrets never appear on screen after they are entered.

## Brand Commitments
Must not feel like the previous Arkived UI ("too AI slop"): gradients, glows, decorative cards, pills, emoji, roomy marketing spacing. Also not like the Azure Portal's blades. ServiceBusExplorer is a feature reference, not a design reference.

## Evidence on Hand
Direction A ("Workbench") mockup chosen by the user on 2026-10-15: `docs/design/m1b-mockup-a.dc.html`. Design spec: `docs/superpowers/specs/2026-10-15-m1b-design-system-shell-design.md`. No testimonials, customers or benchmarks exist; do not invent any.

## Product Principles
- Attention is earned: the interface shows where something needs the operator, and stays quiet elsewhere.
- Density and keyboard speed over discoverability and decoration.
- Production is never ambiguous: the active connection and its environment are always visible.
- Destructive actions are confirmed in context, never in a modal; secrets are never shown again.
