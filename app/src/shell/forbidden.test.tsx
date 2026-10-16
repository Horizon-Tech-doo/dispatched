import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, setTransport, type IpcTransport } from "@/engine";
import { renderShell } from "./test-utils";
import { useShell } from "./context";

const CS = "p-logistics";
const ENTRA = "p-logistics-dev";
const LISTEN_CS = "This key has no Listen right. Use a key whose policy includes Listen (or Manage).";
const MANAGE_CS = "This key has no Manage right. Use a key whose policy includes Manage.";
const LISTEN_ENTRA = /Azure Service Bus Data Receiver role on this namespace or entity\. Owner and Contributor don't include reading messages/;
const MANAGE_ENTRA = /Azure Service Bus Data Owner role on this namespace/;

const seed = (profile: string, id: string, kind: string, subTab: string) => ({
  last_profile: profile,
  [`shell:${profile}`]: { tabs: [{ id, kind, subTab }], activeTab: id, expanded: [], selected: null, layout: { treeSize: 18, drawerOpen: false, drawerSize: 18, drawerTab: "activity" } },
});

/** Make some commands fail with `forbidden`, as the engine words it. */
function deny(mock: IpcTransport, cmds: string[], right: string) {
  setTransport({
    listen: mock.listen,
    invoke: (async (cmd: string, args?: Record<string, unknown>) => {
      if (cmds.includes(cmd)) throw new EngineError("forbidden", `access denied (needs ${right}): Unauthorized access. '${right}' claim(s) are required to perform this operation.`);
      return mock.invoke(cmd, args);
    }) as IpcTransport["invoke"],
  });
}

function SheetProbe() {
  const { sheet } = useShell();
  return <output aria-label="Open sheet">{sheet ? JSON.stringify(sheet) : "none"}</output>;
}

describe("forbidden errors give advice for how the connection signs in", () => {
  it.each([
    [CS, LISTEN_CS],
    [ENTRA, LISTEN_ENTRA],
  ])("a forbidden peek on %s names the fix and offers Edit connection", async (profile, advice) => {
    renderShell("peek-error", seed(profile, "invoice-generate", "queue", "dead-letter"), <SheetProbe />);
    expect(await screen.findByRole("heading", { name: "Can't read the dead-letter queue" })).toBeInTheDocument();
    expect(screen.getByText(advice)).toBeInTheDocument();
    expect(screen.getByText(/access denied \(needs Listen\)/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Edit connection" }));
    expect(screen.getByLabelText("Open sheet")).toHaveTextContent(`"profileId":"${profile}"`);
  });

  it.each([
    [CS, MANAGE_CS],
    [ENTRA, MANAGE_ENTRA],
  ])("the tree's limited banner on %s", async (profile, advice) => {
    renderShell("forbidden", { last_profile: profile });
    await screen.findByRole("heading", { name: "Showing pinned entities only" });
    expect(screen.getByText(advice, { exact: false })).toBeInTheDocument();
  });

  it.each([
    [CS, MANAGE_CS],
    [ENTRA, MANAGE_ENTRA],
  ])("Overview on %s", async (profile, advice) => {
    const { mock } = renderShell("default", seed(profile, "invoice-generate", "queue", "overview"));
    deny(mock, ["entity_details"], "Manage");
    await userEvent.keyboard("{F5}");
    expect(await screen.findByText(advice, { exact: false }, { timeout: 4000 })).toBeInTheDocument();
  });

  it.each([
    [CS, LISTEN_CS],
    [ENTRA, LISTEN_ENTRA],
  ])("Sessions on %s", async (profile, advice) => {
    const { mock } = renderShell("default", seed(profile, "returns-intake", "queue", "sessions"));
    deny(mock, ["list_sessions"], "Listen");
    await userEvent.keyboard("{F5}");
    expect(await screen.findByText(advice, { exact: false }, { timeout: 4000 })).toBeInTheDocument();
  });

  it.each([
    [CS, MANAGE_CS],
    [ENTRA, MANAGE_ENTRA],
  ])("Rules on %s", async (profile, advice) => {
    const { mock } = renderShell("default", seed(profile, "shipment-events/subscriptions/tracking-sync", "subscription", "rules"));
    deny(mock, ["list_rules"], "Manage");
    await userEvent.keyboard("{F5}");
    await waitFor(() => expect(screen.getByText(advice, { exact: false })).toBeInTheDocument(), { timeout: 4000 });
  });

  it.each([
    [CS, MANAGE_CS],
    [ENTRA, MANAGE_ENTRA],
  ])("the Dashboard's per-topic failures on %s", async (profile, advice) => {
    const { mock } = renderShell("default", seed(profile, "::dashboard", "dashboard", "overview"));
    await screen.findByRole("grid", { name: "Dashboard" });
    setTransport({
      listen: mock.listen,
      invoke: (async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd !== "dashboard") return mock.invoke(cmd, args);
        return { rows: [], failures: [{ topic: "orders", kind: "forbidden", message: "access denied (needs Manage): Unauthorized" }], limited: null };
      }) as IpcTransport["invoke"],
    });
    await userEvent.keyboard("{F5}");
    const banner = await screen.findByText(/Couldn't list the subscriptions of 1 topic/);
    expect(within(banner.closest("div")!.parentElement!).getByText(advice, { exact: false })).toBeInTheDocument();
  });
});
