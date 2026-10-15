import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, setTransport } from "@/engine";
import { renderShell } from "./test-utils";

const treePanel = () => document.querySelector('[data-panel-id="tree"]');

/** After the first connection, hold every later `connect` until the test releases (or rejects) it. */
function gateConnect(mock: ReturnType<typeof renderShell>["mock"]) {
  let settle!: (fail?: EngineError) => void;
  const gate = new Promise<void>((resolve, reject) => {
    settle = (fail) => (fail ? reject(fail) : resolve());
  });
  gate.catch(() => {});
  setTransport({
    listen: mock.listen,
    invoke: async <T,>(cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "connect") await gate;
      return mock.invoke<T>(cmd, args);
    },
  });
  return settle;
}

async function chooseConnection(name: RegExp) {
  await screen.findByRole("button", { name: /^Connection: Logistics/ });
  screen.getByRole("button", { name: /^Connection:/ }).focus();
  await userEvent.keyboard("{Enter}");
  await userEvent.click(await screen.findByRole("menuitem", { name }));
}

describe("Shell", () => {
  it("welcomes with saved connections and connects on click", async () => {
    renderShell("default");
    const list = await screen.findByRole("list", { name: "Saved connections" });
    await userEvent.click(within(list).getByRole("button", { name: /Logistics\b.*sb-logistics-prod/ }));
    const bar = screen.getByRole("banner");
    await waitFor(() => expect(within(bar).getByRole("button", { name: "Connection: Logistics, sb-logistics-prod, production" })).toBeInTheDocument());
    expect(within(bar).getByText("PROD")).toBeInTheDocument();
    expect(within(screen.getByRole("contentinfo")).getByText("Peek only, nothing locked")).toBeInTheDocument();
  });

  it("reconnects to the last profile on start", async () => {
    renderShell("default", { last_profile: "p-logistics-dev" });
    await waitFor(() => expect(screen.getByRole("button", { name: /Connection: Logistics dev/ })).toBeInTheDocument());
  });

  it("asks for a first connection when there are none", async () => {
    renderShell("empty");
    expect(await screen.findByRole("heading", { name: "Add a connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Add connection string" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Sign in with Entra ID" })).toBeInTheDocument();
  });

  it("missing secret shows the fix", async () => {
    renderShell("missing-secret", { last_profile: "p-logistics" });
    expect(await screen.findByRole("heading", { name: "Couldn't connect to Logistics" })).toBeInTheDocument();
    expect(screen.getByText(/isn't in the system keychain/)).toHaveAttribute("title", expect.stringContaining("no stored connection string"));
    expect(screen.getByRole("button", { name: "Edit connection" })).toBeInTheDocument();
  });

  it("keeps the workbench mounted while the same profile reconnects", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await waitFor(() => expect(treePanel()).not.toBeNull());
    const release = gateConnect(mock);
    await chooseConnection(/^Logistics sb-logistics-prod/);
    expect(await within(screen.getByRole("contentinfo")).findByText("Connecting…")).toBeInTheDocument();
    expect(treePanel()).not.toBeNull();
    expect(screen.queryByRole("list", { name: "Saved connections" })).not.toBeInTheDocument();
    release();
    await waitFor(() => expect(within(screen.getByRole("contentinfo")).getByText("Peek only, nothing locked")).toBeInTheDocument());
    expect(treePanel()).not.toBeNull();
  });

  it("falls back to the welcome screen when connecting to a different profile", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await waitFor(() => expect(treePanel()).not.toBeNull());
    gateConnect(mock);
    await chooseConnection(/Logistics dev/);
    expect(await screen.findByRole("list", { name: "Saved connections" })).toBeInTheDocument();
    expect(treePanel()).toBeNull();
  });

  it("falls back to the welcome screen when the reconnect fails", async () => {
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await waitFor(() => expect(treePanel()).not.toBeNull());
    const settle = gateConnect(mock);
    await chooseConnection(/^Logistics sb-logistics-prod/);
    await waitFor(() => expect(within(screen.getByRole("contentinfo")).getByText("Connecting…")).toBeInTheDocument());
    settle(new EngineError("network", "network error: connection reset by peer"));
    expect(await screen.findByRole("heading", { name: "Couldn't connect to Logistics" })).toBeInTheDocument();
    expect(treePanel()).toBeNull();
  });
});
