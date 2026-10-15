import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError } from "@/engine";
import { renderShell } from "../test-utils";

/** Open the connection selector from the keyboard (jsdom's resizable panels can swallow the pointer-down). */
async function chooseMenuItem(user: ReturnType<typeof userEvent.setup>, trigger: RegExp, item: string) {
  const button = await screen.findByRole("button", { name: trigger });
  button.focus();
  await user.keyboard("{Enter}");
  await user.click(await screen.findByRole("menuitem", { name: item }));
}

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

  it("keeps the secret out of the page and the mutation cache once it is saved", async () => {
    const user = userEvent.setup();
    const { mock, client } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    const input = within(sheet).getByLabelText("Connection string");
    expect(input).toHaveAttribute("type", "password");
    expect(input).toHaveAttribute("autocomplete", "new-password");
    await user.type(input, CS);
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(document.body.innerHTML).not.toContain("c2VjcmV0");
    // The secret travels in exactly one place: the recorded save call.
    const holders = mock.calls.filter((c) => JSON.stringify(c).includes("c2VjcmV0")).map((c) => c.cmd);
    expect(holders).toEqual(["profile_save"]);
    await waitFor(() => expect(JSON.stringify(client.getMutationCache().getAll().map((m) => m.state.variables))).not.toContain("c2VjcmV0"));
    // A new sheet starts with an empty field.
    await chooseMenuItem(user, /Connection: Orders/, "Add connection…");
    expect(within(await screen.findByRole("dialog", { name: "Add connection" })).getByLabelText("Connection string")).toHaveValue("");
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
    expect(within(sheet).getAllByText("Connection string")).toHaveLength(2); // the read-only method and a labelled value row, not an input
    expect(within(sheet).getByText("sb-logistics-prod.servicebus.windows.net")).toHaveClass("font-mono");
    expect(within(sheet).getByText("Stored in the system keychain.")).toBeInTheDocument();
    expect(within(sheet).queryByLabelText("Connection string")).not.toBeInTheDocument();
    expect(within(sheet).queryByRole("radiogroup", { name: "Sign-in method" })).not.toBeInTheDocument(); // read-only on edit
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    expect(await within(sheet).findByText("Management works")).toBeInTheDocument();
    expect(within(sheet).getByText("Messaging works")).toBeInTheDocument();
    expect(within(sheet).getByText(/listed 4 queues/)).toBeInTheDocument();
    expect(within(sheet).getByText("Tested with saved settings.")).toBeInTheDocument();
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
    const error = within(sheet).getByText("Sign in with Entra ID first.");
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
    // It sits under the sign-in panel, above the Namespace field, not at the top of the form.
    const signInButton = within(sheet).getByRole("button", { name: "Sign in with Entra ID" });
    expect(signInButton.compareDocumentPosition(error) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(error.compareDocumentPosition(within(sheet).getByLabelText("Namespace")) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(sheet).getByLabelText("Name").compareDocumentPosition(error) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });
});

describe("ProfileSheet details", () => {
  async function openEdit(user: ReturnType<typeof userEvent.setup>, name: string) {
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await user.click(within(list).getByRole("button", { name: `Edit ${name}` }));
    return screen.findByRole("dialog", { name: `Edit ${name}` });
  }

  it("shows the sign-in method read-only on edit", async () => {
    const user = userEvent.setup();
    renderShell("default");
    const sheet = await openEdit(user, "Logistics");
    expect(within(sheet).getByText("Sign-in method")).toBeInTheDocument();
    expect(within(sheet).getByText("Connection string", { selector: "span.text-fg" })).toBeInTheDocument();
  });

  it("says no connection string is stored when the connect failed for that reason", async () => {
    const user = userEvent.setup();
    renderShell("missing-secret", { last_profile: "p-logistics" });
    await user.click(await screen.findByRole("button", { name: "Edit connection" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics" });
    expect(within(sheet).getByText("No connection string is stored. Paste it again.")).toBeInTheDocument();
    expect(within(sheet).queryByText("Stored in the system keychain.")).not.toBeInTheDocument();
  });

  it("disables Test connection while the form differs from the saved profile", async () => {
    const user = userEvent.setup();
    renderShell("default");
    const sheet = await openEdit(user, "Logistics");
    const test = within(sheet).getByRole("button", { name: "Test connection" });
    expect(test).toBeEnabled();
    expect(within(sheet).queryByText("Save to test these changes.")).not.toBeInTheDocument();
    await user.type(within(sheet).getByLabelText("Proxy"), "http://p:1");
    expect(test).toBeDisabled();
    expect(within(sheet).getByText("Save to test these changes.")).toBeInTheDocument();
    await user.clear(within(sheet).getByLabelText("Proxy"));
    expect(test).toBeEnabled();
  });

  it("forces WebSockets while a proxy is set", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    const transport = within(sheet).getByRole("combobox", { name: "Transport" });
    expect(transport).toBeEnabled();
    await user.type(within(sheet).getByLabelText("Proxy"), "http://proxy.corp:8080");
    expect(transport).toBeDisabled();
    expect(transport).toHaveTextContent("AMQP over WebSockets");
    expect(within(sheet).getByText("A proxy needs AMQP over WebSockets.")).toBeInTheDocument();
    expect(within(sheet).getByText(/Format http:\/\/host:port\. Leave empty to use HTTPS_PROXY/)).toBeInTheDocument();
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.type(within(sheet).getByLabelText("Connection string"), CS);
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(true));
    const save = mock.calls.find((c) => c.cmd === "profile_save")!.args as { draft: { transport: string; proxy: string } };
    expect(save.draft).toMatchObject({ transport: "websockets", proxy: "http://proxy.corp:8080" });
  });

  it("shows how to finish a device-code sign-in and confirms the copy", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    expect(await within(sheet).findByText("Waiting for you to finish signing in…")).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Copy code" }));
    expect(within(sheet).getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });

  it("offers sign-in when editing an Entra connection and warns when the account is a different one", async () => {
    const user = userEvent.setup();
    renderShell("default");
    const sheet = await openEdit(user, "Logistics dev");
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(sheet).findByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    expect(within(sheet).queryByText(/Sign in with that account to use it/)).not.toBeInTheDocument(); // the profile's own account
    await user.click(within(sheet).getByRole("button", { name: "Use another account" }));
    expect(await within(sheet).findByText(/Signed in as dev@fabrikam.example/)).toBeInTheDocument();
    expect(within(sheet).getByText("This connection uses a different account. Sign in with that account to use it.")).toBeInTheDocument();
  });

  it("shows a failed 'Use another account' without losing the signed-in state", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(sheet).findByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    const invoke = mock.invoke.bind(mock);
    mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "sign_in") throw new EngineError("auth", "The sign-in window was closed before it finished.");
      return invoke(cmd, args);
    }) as typeof mock.invoke;
    await user.click(within(sheet).getByRole("button", { name: "Use another account" }));
    expect(await within(sheet).findByText("The sign-in window was closed before it finished.")).toBeInTheDocument();
    expect(within(sheet).getByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
  });

  it("disconnects before deleting the active connection", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default", { last_profile: "p-logistics" });
    await screen.findByRole("button", { name: /Connection: Logistics, sb-logistics-prod, production/ });
    await chooseMenuItem(user, /Connection: Logistics/, "Manage connections…");
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await user.click(within(list).getByRole("button", { name: "Edit Logistics" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics" });
    await user.click(within(sheet).getByRole("button", { name: "Delete…" }));
    await user.click(within(sheet).getByRole("button", { name: "Delete Logistics" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    const order = mock.calls.map((c) => c.cmd);
    expect(order.lastIndexOf("disconnect")).toBeGreaterThan(-1);
    expect(order.indexOf("disconnect", order.indexOf("connect"))).toBeLessThan(order.indexOf("profile_delete"));
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

  it("marks the active connection as connected instead of offering Connect", async () => {
    const user = userEvent.setup();
    renderShell("default", { last_profile: "p-logistics" });
    await chooseMenuItem(user, /Connection: Logistics/, "Manage connections…");
    const list = await screen.findByRole("dialog", { name: "Connections" });
    expect(within(list).getByText("Connected")).toBeInTheDocument();
    expect(within(list).queryByRole("button", { name: "Connect to Logistics" })).not.toBeInTheDocument();
    expect(within(list).getByRole("button", { name: "Connect to Logistics dev" })).toBeInTheDocument();
  });
});
