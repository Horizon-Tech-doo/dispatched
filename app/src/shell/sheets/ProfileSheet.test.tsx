import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { EngineError, type Profile } from "@/engine";
import { renderShell } from "../test-utils";
import { namespaceField } from "./ProfileSheet";

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
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-prod/ }));
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-logistics-prod.servicebus.windows.net");
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

  it("shows a public namespace by its short name and any other host in full", () => {
    expect(namespaceField("sb-orders.servicebus.windows.net")).toBe("sb-orders");
    expect(namespaceField("myns.privatelink.servicebus.windows.net")).toBe("myns.privatelink.servicebus.windows.net");
    expect(namespaceField("x.servicebus.chinacloudapi.cn")).toBe("x.servicebus.chinacloudapi.cn");
  });

  it.each(["myns.privatelink.servicebus.windows.net", "x.servicebus.chinacloudapi.cn"])(
    "saving an unchanged Entra connection keeps its host %s",
    async (host) => {
      const user = userEvent.setup();
      const { mock, client } = renderShell("default");
      await mock.invoke("sign_in", { tenant: null, deviceCode: false });
      const added = await mock.invoke<Profile>("profile_save", {
        draft: { name: "Private", auth: { kind: "entra", namespace: host }, production: false, transport: "amqp", pinned_entities: [] },
      });
      expect(added.host).toBe(host);
      await client.invalidateQueries({ queryKey: ["profiles"] });
      const sheet = await openEdit(user, "Private");
      expect(within(sheet).getByLabelText("Namespace")).toHaveValue(host);
      await user.click(within(sheet).getByRole("button", { name: "Save" }));
      await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
      const last = mock.calls.filter((c) => c.cmd === "profile_save").at(-1)!.args as { draft: { auth: { namespace: string } } };
      expect(last.draft.auth.namespace).toBe(host);
      const saved = (await mock.invoke<Profile[]>("profiles_list")).find((p) => p.id === added.id)!;
      expect(saved.host).toBe(host);
    },
  );

  it("focuses the first field of a form sheet and the sheet itself otherwise", async () => {
    const user = userEvent.setup();
    renderShell("default");
    await user.click(await screen.findByRole("button", { name: "Add connection…" }));
    const add = await screen.findByRole("dialog", { name: "Add connection" });
    await waitFor(() => expect(within(add).getByLabelText("Name")).toHaveFocus());
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await waitFor(() => expect(list).toHaveFocus());
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

describe("Entra discovery in the Add connection sheet", () => {
  async function openEntra(scenario: Parameters<typeof renderShell>[0] = "empty") {
    const user = userEvent.setup();
    const utils = renderShell(scenario);
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    return { user, sheet, ...utils };
  }
  async function signedIn(scenario: Parameters<typeof renderShell>[0] = "empty") {
    const o = await openEntra(scenario);
    await o.user.click(within(o.sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(o.sheet).findByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    return o;
  }

  it("keeps the typed fields before sign-in and lists tenants and namespaces right after it, without another click", async () => {
    const { user, sheet, mock } = await openEntra();
    expect(within(sheet).getByLabelText("Namespace")).toBeInTheDocument();
    expect(within(sheet).queryByRole("combobox", { name: "Tenant" })).not.toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "list_tenants" || c.cmd === "discover_namespaces")).toBe(false);
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(sheet).findByRole("listbox", { name: "Namespaces" })).toBeInTheDocument();
    expect(mock.calls.filter((c) => c.cmd === "discover_namespaces")).toHaveLength(1);
  });

  it("lists the tenants with the home tenant first and its namespaces", async () => {
    const { user, sheet } = await signedIn();
    const tenant = await within(sheet).findByRole("combobox", { name: "Tenant" });
    expect(tenant).toHaveTextContent("Horizon Tech (home)");
    await user.click(tenant);
    const options = await screen.findAllByRole("option", { name: /Horizon Tech|Northwind/ });
    expect(options.map((o) => o.textContent?.replace(/\s+/g, " ").trim())).toEqual(["Horizon Tech (home)", "Northwind Traders"]);
    await user.keyboard("{Escape}");
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    const rows = within(list).getAllByRole("option");
    expect(rows.map((r) => r.textContent)).toEqual([
      expect.stringMatching(/^sb-finance-eventsFinance shared · northeurope · StandardEntra only$/),
      expect.stringMatching(/^sb-logistics-devLogistics dev · westeurope · Standard$/),
      expect.stringMatching(/^sb-logistics-prodLogistics prod · westeurope · Premium$/),
      expect.stringMatching(/^sb-partner-gatewayPartner integrations · northeurope · Standard$/),
    ]);
    expect(within(sheet).queryByRole("textbox", { name: "Filter namespaces" })).not.toBeInTheDocument(); // 4 rows need no filter
    expect(within(sheet).queryByLabelText("Namespace")).not.toBeInTheDocument(); // typed fields wait behind the toggle
  });

  it("picking a namespace fills Namespace, Tenant and an empty Name, and shows what was filled", async () => {
    const { user, sheet, mock } = await signedIn();
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-prod/ }));
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-logistics-prod.servicebus.windows.net");
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue(""); // home tenant
    expect(within(sheet).getByLabelText("Name")).toHaveValue("sb-logistics-prod");
    expect(within(sheet).getByRole("option", { name: /sb-logistics-prod/ })).toHaveAttribute("aria-selected", "true");
    // A typed name is kept when another namespace is picked.
    await user.clear(within(sheet).getByLabelText("Name"));
    await user.type(within(sheet).getByLabelText("Name"), "Prod logistics");
    await user.click(within(sheet).getByRole("option", { name: /sb-partner-gateway/ }));
    expect(within(sheet).getByLabelText("Name")).toHaveValue("Prod logistics");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-partner-gateway.servicebus.windows.net");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(true));
    const draft = (mock.calls.find((c) => c.cmd === "profile_save")!.args as { draft: { auth: unknown } }).draft;
    expect(draft.auth).toEqual({ kind: "entra", namespace: "sb-partner-gateway.servicebus.windows.net", tenant: null });
  });

  it("marks Entra-only namespaces and says why", async () => {
    const { sheet } = await signedIn();
    const row = await within(sheet).findByRole("option", { name: /sb-finance-events/ });
    const note = within(row).getByText("Entra only");
    expect(note).toHaveAttribute("title", expect.stringMatching(/Shared access keys are turned off/));
    expect(within(sheet).getByRole("option", { name: /sb-logistics-dev/ })).not.toHaveTextContent("Entra only");
  });

  it("selects with the keyboard: arrows move, Enter picks, and Enter does not save the form", async () => {
    const { user, sheet, mock } = await signedIn();
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    list.focus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(list).toHaveAttribute("aria-activedescendant", within(list).getAllByRole("option")[2].id);
    await user.keyboard("{ArrowUp}{Enter}");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-logistics-dev.servicebus.windows.net");
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
    list.focus();
    await user.keyboard("{End}{ }");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-partner-gateway.servicebus.windows.net");
    await user.keyboard("{Home}{Enter}");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-finance-events.servicebus.windows.net");
  });

  it("filters when there are more than eight namespaces", async () => {
    const { user, sheet } = await signedIn("many-namespaces");
    const filter = await within(sheet).findByRole("textbox", { name: "Filter namespaces" });
    expect(within(sheet).getAllByRole("option")).toHaveLength(12);
    await user.type(filter, "team-0");
    expect(within(sheet).getAllByRole("option")).toHaveLength(8);
    await user.clear(filter);
    await user.type(filter, "partner");
    expect(within(sheet).getAllByRole("option")).toHaveLength(1);
    await user.type(filter, "zzz");
    expect(within(sheet).queryByRole("option")).not.toBeInTheDocument();
    expect(within(sheet).getByText(/No namespaces match/)).toBeInTheDocument();
  });

  it("shows the tenant picker as soon as the account is signed in, before the namespaces arrive", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    let release: () => void = () => {};
    const held = new Promise<void>((r) => (release = r));
    const invoke = mock.invoke.bind(mock);
    mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "discover_namespaces") await held;
      return invoke(cmd, args);
    }) as typeof mock.invoke;
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    const tenant = await within(sheet).findByRole("combobox", { name: "Tenant" });
    expect(tenant).toHaveTextContent("Horizon Tech (home)");
    expect(within(sheet).getByRole("status", { name: "Loading namespaces" })).toBeInTheDocument();
    expect(within(sheet).queryByRole("listbox")).not.toBeInTheDocument();
    release();
    expect(await within(sheet).findByRole("listbox", { name: "Namespaces" })).toBeInTheDocument();
  });

  it("offers to sign in to a tenant that needs multi-factor sign-in, then lists its namespaces", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.click(within(sheet).getByRole("combobox", { name: "Tenant" }));
    await user.click(await screen.findByRole("option", { name: "Northwind Traders" }));
    expect(await within(sheet).findByText("Northwind Traders needs its own sign-in")).toBeInTheDocument();
    expect(within(sheet).getByText(/requires multi-factor sign-in/)).toBeInTheDocument();
    expect(within(sheet).queryByRole("listbox")).not.toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Sign in to Northwind Traders" }));
    expect(mock.calls.at(-1)).toMatchObject({ cmd: "sign_in", args: { tenant: "tenant-guest", deviceCode: false } });
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(within(list).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "nw-ordersNorthwind production · westeurope · Premium",
      "nw-telemetryNorthwind shared · uksouth · Standard",
    ]);
    expect(within(sheet).getByRole("combobox", { name: "Tenant" })).toHaveTextContent("Northwind Traders");
    // Picking one files it under that tenant.
    await user.click(within(list).getByRole("option", { name: /nw-orders/ }));
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("tenant-guest");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("nw-orders.servicebus.windows.net");
  });

  it("can sign in to the tenant with a device code, and cancelling returns to the problem", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.click(within(sheet).getByRole("combobox", { name: "Tenant" }));
    await user.click(await screen.findByRole("option", { name: "Northwind Traders" }));
    await user.click(await within(sheet).findByRole("button", { name: "Use a device code instead" }));
    expect(mock.calls.at(-1)).toMatchObject({ cmd: "sign_in", args: { tenant: "tenant-guest", deviceCode: true } });
    expect(await within(sheet).findByText("QX4T-9KLM")).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Cancel sign-in" }));
    expect(await within(sheet).findByRole("button", { name: "Sign in to Northwind Traders" })).toBeInTheDocument();
    expect(within(sheet).queryByText(/cancelled/i)).not.toBeInTheDocument();
  });

  it("offers the manual fields for a tenant that needs multi-factor sign-in, with its id filled in", async () => {
    const { user, sheet } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.click(within(sheet).getByRole("combobox", { name: "Tenant" }));
    await user.click(await screen.findByRole("option", { name: "Northwind Traders" }));
    await user.click(await within(sheet).findByRole("button", { name: "Enter a namespace manually" }));
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("tenant-guest");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("");
    // Back to the home tenant: the list returns.
    await user.click(within(sheet).getByRole("combobox", { name: "Tenant" }));
    await user.click(await screen.findByRole("option", { name: "Horizon Tech (home)" }));
    expect(await within(sheet).findByRole("listbox", { name: "Namespaces" })).toBeInTheDocument();
  });

  it("keeps manual entry one click away and starts the typed fields empty", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.click(within(sheet).getByRole("button", { name: "Enter a namespace manually" }));
    expect(within(sheet).queryByRole("button", { name: "Enter a namespace manually" })).not.toBeInTheDocument();
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("");
    await user.type(within(sheet).getByLabelText("Name"), "Typed");
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-typed");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(true));
  });

  it("asks to pick or type a namespace when saving without one", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.type(within(sheet).getByLabelText("Name"), "Ops");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    expect(within(sheet).getByText("Pick a namespace from the list, or enter one manually.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
  });

  it("says when a tenant has no namespaces and what access is needed", async () => {
    const { sheet } = await signedIn("no-namespaces");
    expect(await within(sheet).findByText("No Service Bus namespaces in this tenant.")).toBeInTheDocument();
    expect(within(sheet).getByText(/You need Reader access on a subscription/)).toBeInTheDocument();
    expect(within(sheet).queryByText(/couldn't be listed/)).not.toBeInTheDocument();
  });

  it("shows a listing failure as a problem with a way to retry", async () => {
    const { user, sheet, mock } = await signedIn("discovery-error");
    // A network error is retried once after a second before it shows.
    expect(await within(sheet).findByText("Couldn't list namespaces", {}, { timeout: 3000 })).toBeInTheDocument();
    expect(within(sheet).getByText(/connection reset by peer/)).toBeInTheDocument();
    const before = mock.calls.filter((c) => c.cmd === "discover_namespaces").length;
    await user.click(within(sheet).getByRole("button", { name: "Try again" }));
    await waitFor(() => expect(mock.calls.filter((c) => c.cmd === "discover_namespaces").length).toBeGreaterThan(before));
  });

  it("shows a loading skeleton in the list shape first", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    expect(await within(sheet).findByRole("status", { name: "Loading namespaces" })).toBeInTheDocument();
    expect(await within(sheet).findByRole("listbox", { name: "Namespaces" })).toBeInTheDocument();
    expect(within(sheet).queryByRole("status", { name: "Loading namespaces" })).not.toBeInTheDocument();
  });

  it("names the subscriptions that could not be listed, with the reasons one click away", async () => {
    const { user, sheet } = await signedIn();
    const summary = await within(sheet).findByText("1 subscription couldn't be listed");
    expect(within(sheet).getByText(/you need Reader on this subscription/i)).not.toBeVisible(); // closed until asked for
    await user.click(summary);
    expect(within(sheet).getByText(/you need Reader on this subscription/i)).toBeInTheDocument();
    expect(within(sheet).getByText("Legacy sandbox")).toBeInTheDocument();
  });

  it("records the listings in the activity log with real plurals", async () => {
    const { sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    const lines: string[] = [];
    const off = await mock.listen<{ action: string }>("activity", (e) => lines.push(e.action));
    await mock.invoke("list_tenants");
    await mock.invoke("discover_namespaces", { tenant: null });
    await mock.invoke("discover_namespaces", { tenant: "tenant-guest" }).catch(() => {});
    off();
    expect(lines).toEqual(["Listed 2 tenants", "Found 4 namespaces in the home tenant", "Finding namespaces in tenant tenant-guest failed"]);
  });

  it("does not offer the picker when editing an Entra connection", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default");
    await mock.invoke("sign_in", { tenant: null, deviceCode: false });
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    const list = await screen.findByRole("dialog", { name: "Connections" });
    await user.click(within(list).getByRole("button", { name: "Edit Logistics dev" }));
    const sheet = await screen.findByRole("dialog", { name: "Edit Logistics dev" });
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-logistics-dev");
    expect(within(sheet).queryByRole("listbox")).not.toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "list_tenants" || c.cmd === "discover_namespaces")).toBe(false);
  });
});
