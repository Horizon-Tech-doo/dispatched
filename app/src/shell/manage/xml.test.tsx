import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import type { EntityRefForm } from "@/engine";
import { renderShell } from "../test-utils";
import { OpenSheet, lastCall } from "../operate/testing";
import type { OperationSeed } from "../operate/seed";

const open = (seed: OperationSeed, scenario: Parameters<typeof renderShell>[0] = "operate-ok") =>
  renderShell(scenario, { last_profile: "p-logistics" }, <OpenSheet sheet={{ kind: "operation", ...seed }} />);

describe("ServiceBusExplorer XML", () => {
  it("export_whole_namespace_without_keys", async () => {
    const { mock } = open({ op: "entities-export", entities: [] });
    const sheet = await screen.findByRole("dialog", { name: "Export entities" });
    expect(within(sheet).getByText(/every queue and topic/i)).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("button", { name: "Export…" }));
    expect(await within(sheet).findByText(/Exported 7 entities to logistics-entities\.xml/)).toBeInTheDocument();
    expect(lastCall<{ entities: EntityRefForm[]; includeKeys: boolean }>(mock, "entities_export")).toMatchObject({ entities: [], includeKeys: false });
  });

  it("include_keys_warns", async () => {
    open({ op: "entities-export", entities: [{ kind: "queue", entity: "label-print" }] });
    const sheet = await screen.findByRole("dialog", { name: "Export entities" });
    expect(within(sheet).queryByText(/Anyone with the file/)).toBeNull();
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /^Include keys/ }));
    expect(within(sheet).getByText(/Anyone with the file can send and receive/)).toBeInTheDocument();
  });

  it("import_preview_then_import_with_overwrite", async () => {
    const { mock } = open({ op: "entities-import" });
    const sheet = await screen.findByRole("dialog", { name: "Import entities" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file…" }));
    const table = await within(sheet).findByRole("table", { name: "Entities in the file" });
    expect(within(table).getByText("orders-import")).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole("checkbox", { name: /^Replace existing entities/ }));
    await userEvent.click(within(sheet).getByRole("button", { name: "Import 3 entities" }));
    await waitFor(() => expect(lastCall<{ overwrite: boolean }>(mock, "op_entities_import")?.overwrite).toBe(true));
  });

  it("import_parse_refusal_shows_core_text", async () => {
    open({ op: "entities-import" }, "import-broken");
    const sheet = await screen.findByRole("dialog", { name: "Import entities" });
    await userEvent.click(within(sheet).getByRole("button", { name: "Choose file…" }));
    expect(await within(sheet).findByText(/is not a TimeSpan/)).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: /^Import/ })).toBeDisabled();
  });
});
