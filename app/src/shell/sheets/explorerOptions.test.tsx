import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
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
});
