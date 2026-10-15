import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { renderShell } from "../test-utils";

const CS = "Endpoint=sb://orders.servicebus.windows.net/;SharedAccessKeyName=reader;SharedAccessKey=c2VjcmV0";

describe("ProfileSheet", () => {
  it("adds a connection-string profile and connects to it", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.type(within(sheet).getByLabelText("Connection string"), CS);
    await user.click(within(sheet).getByRole("checkbox", { name: /Production/ }));
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(await screen.findByRole("button", { name: /Connection: Orders/ })).toBeInTheDocument();
    expect(within(screen.getByRole("banner")).getByText("PROD")).toBeInTheDocument();
  });

  it("keeps the connection string a password field and never renders it back", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    const input = within(sheet).getByLabelText("Connection string");
    expect(input).toHaveAttribute("type", "password");
    await user.type(input, CS);
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(document.body.innerHTML).not.toContain("c2VjcmV0");
  });

  it("validates before saving", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(within(sheet).getByText("Enter a name for this connection.")).toBeInTheDocument();
    expect(within(sheet).getByText("Paste the namespace's connection string.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
  });

  it("shows a duplicate name under the Name field", async () => {
    const user = userEvent.setup();
    renderShell("default");
    await user.click(await screen.findByRole("button", { name: "Add connection…" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.type(within(sheet).getByLabelText("Name"), "logistics");
    await user.type(within(sheet).getByLabelText("Connection string"), CS);
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(await within(sheet).findByText(/already exists/)).toBeInTheDocument();
    expect(within(sheet).getByLabelText("Name")).toHaveAttribute("aria-invalid", "true");
  });

  it("edits without ever showing the stored secret, tests, and deletes after confirming", async () => {
    const user = userEvent.setup();
    renderShell("default");
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await user.click(within(list).getByRole("button", { name: "Edit Logistics" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics" });
    expect(within(sheet).getByText("Stored in the system keychain.")).toBeInTheDocument();
    expect(within(sheet).queryByLabelText("Connection string")).not.toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    expect(await within(sheet).findByText(/listed 4 queue/)).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Delete…" }));
    await user.click(within(sheet).getByRole("button", { name: "Delete Logistics" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Edit Logistics" })).not.toBeInTheDocument());
    expect(screen.queryByRole("button", { name: /^Logistics\b.*sb-logistics-prod/ })).not.toBeInTheDocument();
  });

  it("keeps the profile when the delete confirmation is declined", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default");
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await user.click(within(list).getByRole("button", { name: "Edit Logistics" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics" });
    await user.click(within(sheet).getByRole("button", { name: "Delete…" }));
    expect(within(sheet).getByText(/Its saved connection string is removed from the keychain too/)).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Keep" }));
    expect(within(sheet).getByRole("button", { name: "Delete…" })).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_delete")).toBe(false);
  });

  it("signs in with a device code and then adds an Entra profile", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    expect(await within(sheet).findByText("QX4T-9KLM")).toBeInTheDocument();
    expect(await within(sheet).findByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    await user.type(within(sheet).getByLabelText("Name"), "Ops");
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-ops");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(await screen.findByRole("button", { name: /Connection: Ops/ })).toBeInTheDocument();
  });

  it("returns to the sign-in buttons without an error when sign-in is cancelled", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(sheet).findByText("Finish signing in in your browser.")).toBeInTheDocument(); // the sign-in is waiting
    await user.click(within(sheet).getByRole("button", { name: "Cancel sign-in" }));
    expect(await within(sheet).findByRole("button", { name: "Sign in with Entra ID" })).toBeInTheDocument();
    expect(within(sheet).queryByText(/cancelled/i)).not.toBeInTheDocument();
    expect(within(sheet).queryByText(/Signed in as/)).not.toBeInTheDocument();
  });

  it("asks to sign in before saving an Entra profile", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.type(within(sheet).getByLabelText("Name"), "Ops");
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-ops");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(within(sheet).getByText("Sign in with Entra ID first.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
  });
});

describe("ProfilesSheet", () => {
  it("lists connections with their hosts and a PROD mark", async () => {
    const user = userEvent.setup();
    renderShell("default");
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    expect(within(list).getByRole("button", { name: "Edit Logistics" })).toBeInTheDocument();
    expect(within(list).getAllByText("PROD").length).toBeGreaterThan(0);
    await user.click(within(list).getByRole("button", { name: "Add connection" }));
    expect(await screen.findByRole("dialog", { name: "Add connection" })).toBeInTheDocument();
  });
});
