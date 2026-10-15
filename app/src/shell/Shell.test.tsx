import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "./test-utils";

describe("Shell", () => {
  it("welcomes with saved connections and connects on click", async () => {
    renderShell("default");
    const list = await screen.findByRole("list", { name: "Saved connections" });
    await userEvent.click(within(list).getByRole("button", { name: /Logistics\b.*sb-logistics-prod/ }));
    const bar = screen.getByRole("banner");
    await waitFor(() => expect(within(bar).getByRole("button", { name: /Connection: Logistics/ })).toBeInTheDocument());
    expect(within(bar).getByText("PROD")).toBeInTheDocument();
    expect(within(screen.getByRole("contentinfo")).getByText("Connected")).toBeInTheDocument();
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
    expect(screen.getByText(/no stored connection string/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Edit connection" })).toBeInTheDocument();
  });
});
