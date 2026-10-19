import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import * as highlight from "../tree/highlight";
import { renderShell } from "../test-utils";

// jsdom lays every element out at 0x0, so the resizable panels swallow pointer-downs: focus the box before typing.
const typeInto = async (el: HTMLElement, text: string) => {
  await act(async () => el.focus());
  await userEvent.keyboard(text);
};

const saved = (mock: { calls: { cmd: string; args?: unknown }[] }) =>
  mock.calls.filter((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "shell:p-logistics").at(-1)?.args as
    | { value: { explorer: { rules: unknown[]; counts: string[]; openOn: string } } }
    | undefined;

describe("explorer options", () => {
  it("adds a name rule, rejects a bad pattern, and saves", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Explorer options" }));
    const sheet = screen.getByRole("dialog", { name: "Explorer options for Logistics" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    const value = within(sheet).getByLabelText("Rule 1 value");
    await typeInto(value, "(");
    expect(await within(sheet).findByText(/Not a valid pattern/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Save" })).toBeDisabled();
    await act(async () => value.focus());
    await userEvent.clear(value);
    await typeInto(value, "^invoice");
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved(mock)?.value.explorer.rules).toHaveLength(1));
  });

  it("saves the counts and the sub-tab entities open on", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Explorer options" }));
    const sheet = screen.getByRole("dialog", { name: "Explorer options for Logistics" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Scheduled" }));
    await userEvent.click(within(sheet).getByRole("radio", { name: "Messages" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(saved(mock)?.value.explorer.counts).toEqual(["active", "scheduled", "dead-letter"]));
    expect(saved(mock)?.value.explorer.openOn).toBe("messages");
  });

  it("keeps at least one count on", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Explorer options" }));
    const sheet = screen.getByRole("dialog", { name: "Explorer options for Logistics" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Active" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Dead-letter" }));
    expect(within(sheet).getByRole("button", { name: "Active" })).toHaveAttribute("aria-pressed", "true");
  });

  it("opens from the palette and rejects a count rule that is not a number", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.keyboard("{Control>}k{/Control}");
    await typeInto(await screen.findByPlaceholderText("Go to entity or command"), "Explorer options");
    await userEvent.keyboard("{Enter}");
    const sheet = await screen.findByRole("dialog", { name: "Explorer options for Logistics" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await userEvent.click(within(sheet).getByRole("combobox", { name: "Rule 1 match" }));
    await userEvent.click(await screen.findByRole("option", { name: "Dead-letter at least" }));
    const value = within(sheet).getByLabelText("Rule 1 value");
    await act(async () => value.focus());
    await userEvent.clear(value);
    expect(await within(sheet).findByText("Enter a whole number, 0 or more.")).toBeInTheDocument();
    await typeInto(value, "100");
    expect(within(sheet).queryByText("Enter a whole number, 0 or more.")).not.toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("shows a catastrophic-looking pattern as accepted, since it cannot hang", async () => {
    renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Explorer options" }));
    const sheet = screen.getByRole("dialog", { name: "Explorer options for Logistics" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await typeInto(within(sheet).getByLabelText("Rule 1 value"), "^(a+)+$");
    expect(within(sheet).getByRole("button", { name: "Save" })).toBeEnabled();
  });

  const open = async () => {
    const r = renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("tree", { name: "Namespace" });
    await userEvent.click(screen.getByRole("button", { name: "Explorer options" }));
    return { ...r, sheet: screen.getByRole("dialog", { name: "Explorer options for Logistics" }) };
  };

  it("numbers the rules and moves them up and down", async () => {
    const { sheet, mock } = await open();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await typeInto(within(sheet).getByLabelText("Rule 1 value"), "alpha");
    await typeInto(within(sheet).getByLabelText("Rule 2 value"), "beta");
    expect(within(sheet).getByRole("group", { name: "Rule 2" })).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Move rule 1 up" })).toBeDisabled();
    expect(within(sheet).getByRole("button", { name: "Move rule 2 down" })).toBeDisabled();
    await userEvent.click(within(sheet).getByRole("button", { name: "Move rule 2 up" }));
    expect(within(sheet).getByLabelText("Rule 1 value")).toHaveValue("beta");
    expect(within(sheet).getByLabelText("Rule 2 value")).toHaveValue("alpha");
    await userEvent.click(within(sheet).getByRole("button", { name: "Move rule 1 down" }));
    expect(within(sheet).getByLabelText("Rule 1 value")).toHaveValue("alpha");
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect((saved(mock)?.value.explorer.rules as { match: { pattern: string } }[]).map((r) => r.match.pattern)).toEqual(["alpha", "beta"]));
  });

  it("shows an empty pattern as a problem only after leaving the box or trying to save, and a bad pattern at once", async () => {
    const { sheet, mock } = await open();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    expect(within(sheet).queryByText(/Enter a pattern/)).toBeNull();
    expect(within(sheet).getByRole("button", { name: "Save" })).toBeEnabled();
    await userEvent.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(await within(sheet).findByText(/Enter a pattern/)).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "shell:p-logistics" && "explorer" in (c.args as { value: object }).value)).toBe(false);
    expect(within(sheet).getByRole("button", { name: "Save" })).toBeDisabled();
    // Another rule, left by tabbing away.
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    const second = within(sheet).getByLabelText("Rule 2 value");
    await act(async () => second.focus());
    expect(within(sheet).getAllByText(/Enter a pattern/)).toHaveLength(1);
    await act(async () => second.blur());
    expect(await within(sheet).findAllByText(/Enter a pattern/)).toHaveLength(2);
  });

  it("says how many loaded entities a valid rule matches", async () => {
    const { sheet } = await open();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    const value = within(sheet).getByLabelText("Rule 1 value");
    await typeInto(value, "^invoice");
    expect(await within(sheet).findByText("Matches 1 loaded entity")).toBeInTheDocument();
    await act(async () => value.focus());
    await userEvent.clear(value);
    await typeInto(value, "^(");
    expect(within(sheet).queryByText(/Matches/)).toBeNull();
    await act(async () => value.focus());
    await userEvent.clear(value);
    await typeInto(value, "zzzz");
    expect(await within(sheet).findByText("Matches no loaded entities")).toBeInTheDocument();
    await act(async () => value.focus());
    await userEvent.clear(value);
    await typeInto(value, "e");
    expect(await within(sheet).findByText(/^Matches \d+ loaded entities$/)).toBeInTheDocument();
  });

  it("counts the loaded entities that a count rule reaches", async () => {
    const { sheet } = await open();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await userEvent.click(within(sheet).getByRole("combobox", { name: "Rule 1 match" }));
    await userEvent.click(await screen.findByRole("option", { name: "Dead-letter at least" }));
    const value = within(sheet).getByLabelText("Rule 1 value");
    await act(async () => value.focus());
    await userEvent.clear(value);
    await typeInto(value, "100");
    expect(await within(sheet).findByText("Matches 1 loaded entity")).toBeInTheDocument(); // returns-intake
  });

  it("marks the counts that are on with a check", async () => {
    const { sheet } = await open();
    const active = within(sheet).getByRole("button", { name: "Active" });
    expect(active.querySelector("svg")).not.toBeNull();
    expect(within(sheet).getByRole("button", { name: "Scheduled" }).querySelector("svg")).toBeNull();
  });

  it("compiles a rule once per pattern, not again for another rule's keystroke", async () => {
    const spy = vi.spyOn(highlight, "compileRules");
    const { sheet } = await open();
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Add rule" }));
    await typeInto(within(sheet).getByLabelText("Rule 1 value"), "alpha");
    await typeInto(within(sheet).getByLabelText("Rule 2 value"), "be");
    // The sheet compiles one rule at a time; the tree compiles the saved list (empty here).
    const compiled = () => spy.mock.calls.filter((c) => c[0].length === 1).map((c) => (c[0][0].match as { pattern: string }).pattern);
    const alphaBefore = compiled().filter((p) => p === "alpha").length;
    await userEvent.keyboard("ta"); // two more keystrokes in rule 2
    expect(compiled().filter((p) => p === "alpha").length).toBe(alphaBefore);
    expect(compiled()).toContain("beta");
    expect(alphaBefore).toBe(1);
    spy.mockRestore();
  });
});
