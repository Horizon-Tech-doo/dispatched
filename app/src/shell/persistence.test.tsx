import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { createQueryClient, setTransport } from "@/engine";
import { createMockTransport } from "@/engine/mock/transport";
import { TooltipProvider } from "@/ui";
import { ShellProvider, useShell } from "./context";

function Probe() {
  const { shell, shellLoaded, dispatch, conn } = useShell();
  return (
    <div>
      <button onClick={() => void conn.connect("p-logistics")}>connect prod</button>
      <button onClick={() => void conn.connect("p-logistics-dev")}>connect dev</button>
      <button onClick={() => dispatch({ type: "open", tab: { id: "orders", kind: "queue" } })}>open orders</button>
      <output aria-label="status">{conn.status}</output>
      <output aria-label="loaded">{String(shellLoaded)}</output>
      <output aria-label="tabs">{shell.tabs.map((t) => t.id).join(",")}</output>
    </div>
  );
}

function setup() {
  const mock = createMockTransport("default", 0, {});
  setTransport(mock);
  render(
    <QueryClientProvider client={createQueryClient()}>
      <TooltipProvider>
        <ShellProvider>
          <Probe />
        </ShellProvider>
      </TooltipProvider>
    </QueryClientProvider>,
  );
  return mock;
}

const connect = async (name: string) => {
  await userEvent.click(screen.getByRole("button", { name }));
  await waitFor(() => expect(screen.getByLabelText("status")).toHaveTextContent("connected"));
  await waitFor(() => expect(screen.getByLabelText("loaded")).toHaveTextContent("true"));
};

describe("per-profile shell state", () => {
  it("keeps unsaved state when the same profile reconnects", async () => {
    setup();
    await connect("connect prod");
    await userEvent.click(screen.getByRole("button", { name: "open orders" }));
    expect(screen.getByLabelText("tabs")).toHaveTextContent("orders");
    await connect("connect prod");
    expect(screen.getByLabelText("tabs")).toHaveTextContent("orders");
  });

  it("saves pending state before switching to another profile, and restores it later", async () => {
    const mock = setup();
    await connect("connect prod");
    await userEvent.click(screen.getByRole("button", { name: "open orders" }));
    await connect("connect dev");
    expect(screen.getByLabelText("tabs")).toBeEmptyDOMElement();
    expect(mock.calls.some((c) => c.cmd === "settings_set" && (c.args as { key: string }).key === "shell:p-logistics")).toBe(true);
    await connect("connect prod");
    expect(screen.getByLabelText("tabs")).toHaveTextContent("orders");
  });
});
