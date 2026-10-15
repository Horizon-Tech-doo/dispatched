import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { useShell } from "./context";
import { renderShell } from "./test-utils";

function SheetProbe() {
  const { sheet } = useShell();
  return <output aria-label="sheet">{sheet?.kind ?? "none"}</output>;
}

// In jsdom the resizable panels swallow pointer-downs, so focus moves explicitly (as in the tree tests).
const focus = (el: HTMLElement) => act(() => el.focus());
const tab = (id: string) => ({ id, kind: "queue", subTab: "overview" });
const seed = (ids: string[], activeTab = ids[0]) => ({ last_profile: "p-logistics", "shell:p-logistics": { tabs: ids.map(tab), activeTab } });
const mod = (key: string) => `{Control>}${key}{/Control}`;
const PLACEHOLDER = "Go to entity or command";

describe("hotkeys", () => {
  it("mod+1..9 activate the tab at that position", async () => {
    renderShell("default", seed(["label-print", "customs-docs", "invoice-generate"]));
    await screen.findByRole("tab", { name: /customs-docs/ });
    await userEvent.keyboard(mod("2"));
    expect(screen.getByRole("tab", { name: /customs-docs/ })).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard(mod("3"));
    expect(screen.getByRole("tab", { name: /invoice-generate/ })).toHaveAttribute("aria-selected", "true");
    await userEvent.keyboard(mod("9")); // no ninth tab: unchanged
    expect(screen.getByRole("tab", { name: /invoice-generate/ })).toHaveAttribute("aria-selected", "true");
  });

  it("mod+w closes the active tab", async () => {
    renderShell("default", seed(["label-print", "customs-docs"]));
    await screen.findByRole("tab", { name: /label-print/ });
    await userEvent.keyboard(mod("w"));
    await waitFor(() => expect(screen.queryByRole("tab", { name: /label-print/ })).not.toBeInTheDocument());
    expect(screen.getByRole("tab", { name: /customs-docs/ })).toHaveAttribute("aria-selected", "true");
  });

  it("mod+f focuses the tree filter, also from the tree", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    const tree = await screen.findByRole("tree", { name: "Namespace" });
    await focus(tree);
    await userEvent.keyboard(mod("f"));
    expect(screen.getByRole("searchbox", { name: "Filter entities" })).toHaveFocus();
  });

  it("mod+, opens settings, also while typing", async () => {
    renderShell("default", { last_profile: "p-logistics" }, <SheetProbe />);
    await focus(await screen.findByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard(mod(","));
    expect(screen.getByLabelText("sheet")).toHaveTextContent("settings");
  });

  it("mod+, closes the palette before opening settings", async () => {
    renderShell("default", { last_profile: "p-logistics" }, <SheetProbe />);
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard(mod("k"));
    await screen.findByPlaceholderText(PLACEHOLDER);
    await userEvent.keyboard(mod(","));
    await waitFor(() => expect(screen.queryByPlaceholderText(PLACEHOLDER)).not.toBeInTheDocument());
    expect(screen.getByLabelText("sheet")).toHaveTextContent("settings");
  });

  it("leaves plain keys and tab shortcuts alone while typing in a field", async () => {
    renderShell("default", seed(["label-print", "customs-docs"]));
    await screen.findByRole("tab", { name: /label-print/ });
    await focus(screen.getByRole("searchbox", { name: "Filter entities" }));
    await userEvent.keyboard(mod("2"));
    expect(screen.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true");
  });

  it("does not change tabs behind the palette", async () => {
    renderShell("default", seed(["label-print", "customs-docs"]));
    await screen.findByRole("tab", { name: /label-print/ });
    await userEvent.keyboard(mod("k"));
    await screen.findByPlaceholderText(PLACEHOLDER);
    await userEvent.keyboard(mod("w"));
    await userEvent.keyboard(mod("2"));
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByPlaceholderText(PLACEHOLDER)).not.toBeInTheDocument());
    expect(screen.getAllByRole("tab", { name: /label-print|customs-docs/ })).toHaveLength(2);
    expect(screen.getByRole("tab", { name: /label-print/ })).toHaveAttribute("aria-selected", "true");
  });

  it("moves focus to the tree after the last tab is closed with mod+w", async () => {
    renderShell("default", seed(["label-print"]));
    const strip = await screen.findByRole("tablist", { name: "Open entities" });
    await focus(within(strip).getByRole("tab", { name: /label-print/ }));
    await userEvent.keyboard(mod("w"));
    expect(await screen.findByRole("heading", { name: "Open an entity" })).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("tree", { name: "Namespace" })).toHaveFocus());
  });
});
