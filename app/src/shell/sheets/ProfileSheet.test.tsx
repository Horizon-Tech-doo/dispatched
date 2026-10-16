import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
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

/** The test result region once something has landed in it (it is always in the sheet, so it can announce). */
const testResult = async (sheet: HTMLElement) => {
  const status = within(sheet).getByRole("status");
  await waitFor(() => expect(status).not.toBeEmptyDOMElement());
  return status;
};

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
    expect(within(sheet).getByRole("status")).toHaveTextContent("Tested sb-logistics-prod.servicebus.windows.net as SAS key reader, AMQP.");
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
    expect(within(sheet).getByRole("group", { name: "Selected namespace" })).toHaveTextContent("sb-logistics-prod.servicebus.windows.net");
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
  it("tests a new connection before it is saved and never shows the string again", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.type(within(sheet).getByLabelText("Connection string"), CS);
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    expect(await testResult(sheet)).toHaveTextContent("Tested orders.servicebus.windows.net as SAS key reader, AMQP.");
    expect(within(sheet).getByText(/Management works/)).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_test_draft")).toBe(true);
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
    expect(within(sheet).queryByText(/Save to test/)).toBeNull();
    expect(document.body.textContent).not.toContain("c2VjcmV0");
  });

  it("says so when the sign-in page could not be opened", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    const invoke = mock.invoke.bind(mock);
    mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "open_sign_in_page") throw new EngineError("invalid", "The sign-in page is not an https address, so it was not opened.");
      return invoke(cmd, args);
    }) as typeof mock.invoke;
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    await user.click(await within(sheet).findByRole("button", { name: "Open sign-in page" }));
    expect(await within(sheet).findByText("Couldn't open the page. Open the address above.")).toBeInTheDocument();
  });

  it("device-code sign-in can open the sign-in page", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    await user.click(await within(sheet).findByRole("button", { name: "Open sign-in page" }));
    expect(mock.calls.some((c) => c.cmd === "open_sign_in_page")).toBe(true);
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

  it("tests an edit as edited, without saving it, and drops a result the form no longer matches", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default");
    const sheet = await openEdit(user, "Logistics");
    const test = within(sheet).getByRole("button", { name: "Test connection" });
    expect(test).toBeEnabled();
    await user.clear(within(sheet).getByLabelText("Name"));
    await user.type(within(sheet).getByLabelText("Name"), "Logistics EU");
    await user.type(within(sheet).getByLabelText("Proxy"), "http://p:1");
    expect(test).toBeEnabled();
    expect(within(sheet).queryByText(/Save to test/)).not.toBeInTheDocument();
    await user.click(test);
    expect(await testResult(sheet)).toHaveTextContent("AMQP over WebSockets");
    const sent = mock.calls.find((c) => c.cmd === "profile_test_draft")!.args as { draft: { id: string; name: string; proxy: string; auth: { connection_string: string | null } } };
    expect(sent.draft).toMatchObject({ id: "p-logistics", name: "Logistics EU", proxy: "http://p:1" });
    expect(sent.draft.auth.connection_string).toBeNull(); // the engine uses the stored string
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
    await user.type(within(sheet).getByLabelText("Proxy"), "0");
    expect(within(sheet).getByRole("status")).toBeEmptyDOMElement();
  });

  it("tests an incomplete form by showing what is missing, like Save", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    const test = within(sheet).getByRole("button", { name: "Test connection" });
    expect(test).toBeEnabled();
    await user.click(test);
    expect(within(sheet).getByText("Enter a name for this connection.")).toBeInTheDocument();
    expect(within(sheet).getByText("Paste the namespace's connection string.")).toBeInTheDocument();
    expect(mock.calls.some((c) => c.cmd === "profile_test_draft")).toBe(false);
  });

  it("shows a failed check as a problem with a next step and the raw message, and a skipped one as not tested", async () => {
    const user = userEvent.setup();
    renderShell("forbidden");
    const sheet = await openEdit(user, "Logistics");
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    const status = await testResult(sheet);
    expect(status).toHaveTextContent("Tested sb-logistics-prod.servicebus.windows.net as SAS key reader, AMQP.");
    const alert = within(status).getByRole("alert");
    expect(within(alert).getByText("Management failed")).toBeInTheDocument();
    expect(within(alert).getByText(/needs the Manage right|Give it the Manage right/)).toBeInTheDocument();
    expect(within(alert).getByText(/access denied \(needs Manage\)/)).toHaveClass("font-mono");
    expect(within(status).getByText("Messaging not tested")).toBeInTheDocument();
    expect(within(status).getByText(/the Management check failed/)).toBeInTheDocument();
  });

  it("words the skipped check for the app, and shows other reasons as they come", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("forbidden");
    const sheet = await openEdit(user, "Logistics");
    expect(within(sheet).getByRole("status")).toBeEmptyDOMElement(); // there before any result, so a result is announced
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    const status = await testResult(sheet);
    expect(within(status).getByText("No queue could be listed because the Management check failed. Pin an entity to test receive rights.")).toBeInTheDocument();
    expect(status).not.toHaveTextContent("pass an entity");
    // The other pinned reason, and an unknown one.
    for (const [reason, shown] of [
      ["no entity to probe; pass one", "No entity to test. Pin one to test receive rights."],
      ["the namespace is paused", "The namespace is paused."],
    ]) {
      const invoke = mock.invoke.bind(mock);
      mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
        const r = await invoke(cmd, args);
        return cmd === "profile_test_draft" ? { ...(r as object), management: { status: "ok", detail: "listed 4 queues (Manage)" }, messaging: { status: "skipped", reason } } : r;
      }) as typeof mock.invoke;
      await user.type(within(sheet).getByLabelText("Proxy"), "x"); // a changed form clears the old result
      await user.clear(within(sheet).getByLabelText("Proxy"));
      await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
      await waitFor(() => expect(within(sheet).getByRole("status")).toHaveTextContent(shown));
    }
  });

  it("drops a result when the connection's sign-in is signed in again", async () => {
    const user = userEvent.setup();
    const { mock, client } = renderShell("default");
    await mock.invoke("sign_in", { tenant: null, deviceCode: false });
    const sheet = await openEdit(user, "Logistics dev");
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    await testResult(sheet);
    // Same account, same sign-in, but its `live` flag flips as the engine reports it.
    const invoke = mock.invoke.bind(mock);
    mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => {
      const r = await invoke(cmd, args);
      return cmd === "profiles_list"
        ? (r as { id: string; bound_sign_in: { live: boolean } | null }[]).map((p) => (p.bound_sign_in ? { ...p, bound_sign_in: { ...p.bound_sign_in, live: false } } : p))
        : r;
    }) as typeof mock.invoke;
    await client.invalidateQueries({ queryKey: ["profiles"] });
    await waitFor(() => expect(within(sheet).getByRole("status")).toBeEmptyDOMElement());
  });

  it("brings a new result into view, and drops it when the account changes", async () => {
    const user = userEvent.setup();
    const scroll = vi.spyOn(Element.prototype, "scrollIntoView");
    renderShell("default");
    const sheet = await openEdit(user, "Logistics dev");
    await within(sheet).findByText(/Signed in as|Sign in with Entra ID/);
    if (!within(sheet).queryByText(/Signed in as/)) await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    await within(sheet).findByText(/Signed in as ops@contoso.example/);
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    const status = await testResult(sheet);
    expect(status).toHaveTextContent("Entra ID");
    expect(scroll).toHaveBeenCalledWith({ block: "nearest" });
    await user.click(within(sheet).getByRole("button", { name: "Use another account" }));
    await within(sheet).findByText(/Signed in as dev@fabrikam.example/);
    expect(within(sheet).getByRole("status")).toBeEmptyDOMElement();
    scroll.mockRestore();
  });

  it("keeps a result while the account stays the same", async () => {
    const user = userEvent.setup();
    const { client } = renderShell("default");
    const sheet = await openEdit(user, "Logistics");
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    await testResult(sheet);
    await client.invalidateQueries({ queryKey: ["account"] });
    await client.invalidateQueries({ queryKey: ["profiles"] });
    expect(within(sheet).getByRole("status")).not.toBeEmptyDOMElement();
  });

  it("disables Test while a sign-in is waiting and says why", async () => {
    const user = userEvent.setup();
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    const test = within(sheet).getByRole("button", { name: "Test connection" });
    expect(test).toBeDisabled();
    expect(within(sheet).getByText("Finish signing in to test.")).toBeInTheDocument();
    expect(test).toHaveAccessibleDescription("Finish signing in to test.");
    await within(sheet).findByText(/Signed in as/);
    expect(test).toBeEnabled();
    expect(within(sheet).queryByText("Finish signing in to test.")).not.toBeInTheDocument();
  });

  it("does not keep the typed connection string in the test mutation", async () => {
    const user = userEvent.setup();
    const { client } = renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Add connection string" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.type(within(sheet).getByLabelText("Name"), "Orders");
    await user.type(within(sheet).getByLabelText("Connection string"), CS);
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    await testResult(sheet);
    await waitFor(() => expect(JSON.stringify(client.getMutationCache().getAll().map((m) => m.state))).not.toContain("c2VjcmV0"));
  });

  it("tells an edited Entra connection whose sign-in is gone to save first, when another account is signed in", async () => {
    const user = userEvent.setup();
    const { mock, client } = renderShell("tenant-sign-in");
    await screen.findByRole("button", { name: /Connection: Choose a connection/ });
    await mock.invoke("sign_in", { tenant: null, deviceCode: false }); // the other account becomes current
    await mock.invoke("sign_out", { id: "oid-1" });
    await client.invalidateQueries();
    const sheet = await openEdit(user, "Northwind orders");
    await within(sheet).findByText(/Signed in as dev@fabrikam.example/);
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    const alert = await within(sheet).findByRole("alert");
    expect(within(alert).getByText("Couldn't test the connection")).toBeInTheDocument();
    expect(within(alert).getByText(/Save the connection to use your current sign-in/)).toBeInTheDocument();
  });

  it("does not say to save when nobody is signed in", async () => {
    const user = userEvent.setup();
    const { mock, client } = renderShell("tenant-sign-in");
    await screen.findByRole("button", { name: /Connection: Choose a connection/ });
    await mock.invoke("sign_out", { id: "oid-1" });
    await client.invalidateQueries();
    const sheet = await openEdit(user, "Northwind orders");
    await user.click(within(sheet).getByRole("button", { name: "Test connection" }));
    const alert = await within(sheet).findByRole("alert");
    expect(within(alert).getByText("Couldn't test the connection")).toBeInTheDocument();
    expect(within(alert).queryByText(/Save the connection/)).not.toBeInTheDocument();
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

  it("says Copied only after the clipboard took the code", async () => {
    const user = userEvent.setup();
    vi.spyOn(navigator.clipboard, "writeText").mockRejectedValueOnce(new Error("denied"));
    renderShell("empty");
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Use a device code instead" }));
    await user.click(await within(sheet).findByRole("button", { name: "Copy code" }));
    expect(within(sheet).getByRole("button", { name: "Copy code" })).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Copy code" }));
    expect(await within(sheet).findByRole("button", { name: "Copied" })).toBeInTheDocument();
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
    expect(within(sheet).getByText("This connection uses a different account, ops@contoso.example. Sign in with that account to use it.")).toBeInTheDocument();
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
  type U = ReturnType<typeof userEvent.setup>;
  const cells = (row: HTMLElement) => Array.from(row.children).map((c) => c.textContent);
  const summary = (sheet: HTMLElement) => within(sheet).getByRole("group", { name: "Selected namespace" });
  const showManual = (user: U, sheet: HTMLElement) => user.click(within(sheet).getByRole("button", { name: "Enter a namespace manually" }));
  async function chooseTenant(user: U, sheet: HTMLElement, name: string) {
    await user.click(within(sheet).getByRole("combobox", { name: "Tenant" }));
    await user.click(await screen.findByRole("option", { name }));
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

  it("lists the tenants with the home tenant first, and the namespaces in fixed columns", async () => {
    const { user, sheet } = await signedIn();
    const tenant = await within(sheet).findByRole("combobox", { name: "Tenant" });
    expect(tenant).toHaveTextContent("Horizon Tech (home)");
    await user.click(tenant);
    const options = await screen.findAllByRole("option", { name: /Horizon Tech|Northwind/ });
    expect(options.map((o) => o.textContent?.replace(/\s+/g, " ").trim())).toEqual(["Horizon Tech (home)", "Northwind Traders"]);
    await user.keyboard("{Escape}");
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    const rows = within(list).getAllByRole("option");
    expect(rows.map(cells)).toEqual([
      ["sb-finance-events", "Finance shared", "northeurope", "Standard"],
      ["sb-logistics-dev", "Logistics dev", "westeurope", "Standard"],
      ["sb-logistics-prod", "Logistics prod", "westeurope", "Premium"],
      ["sb-partner-gateway", "Partner integrations", "northeurope", "Standard"],
    ]);
    expect(rows[2]).toHaveAccessibleName("sb-logistics-prod, Logistics prod, westeurope, Premium");
    expect(within(list).queryByText("Entra only")).not.toBeInTheDocument();
    expect(within(sheet).queryByRole("textbox", { name: "Filter namespaces" })).not.toBeInTheDocument(); // 4 rows need no filter
    expect(within(sheet).queryByLabelText("Namespace")).not.toBeInTheDocument(); // typed fields wait behind the toggle
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

  it("picking a namespace collapses to a summary and fills the form; Change brings the list back", async () => {
    const { user, sheet, mock } = await signedIn();
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-prod/ }));
    const sum = summary(sheet);
    expect(within(sum).getByText("sb-logistics-prod.servicebus.windows.net")).toHaveClass("font-mono");
    expect(sum).toHaveTextContent("Horizon Tech (home)");
    expect(within(sheet).queryByRole("listbox")).not.toBeInTheDocument();
    expect(within(sheet).queryByLabelText("Namespace")).not.toBeInTheDocument(); // no raw inputs
    expect(within(sheet).getByLabelText("Name")).toHaveValue("sb-logistics-prod");
    // The typed fields hold what was picked, mono, with the home tenant empty.
    await showManual(user, sheet);
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-logistics-prod.servicebus.windows.net");
    expect(within(sheet).getByLabelText("Namespace")).toHaveClass("font-mono");
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("");
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveAttribute("placeholder", "Home tenant");
    // Change shows the list again with the pick marked; a typed name is kept when another is picked.
    await user.clear(within(sheet).getByLabelText("Name"));
    await user.type(within(sheet).getByLabelText("Name"), "Prod logistics");
    await user.click(within(sheet).getByRole("button", { name: "Change" }));
    expect(within(await within(sheet).findByRole("listbox", { name: "Namespaces" })).getByRole("option", { name: /sb-logistics-prod/ })).toHaveAttribute("aria-selected", "true");
    await user.click(within(sheet).getByRole("option", { name: /sb-partner-gateway/ }));
    expect(summary(sheet)).toHaveTextContent("sb-partner-gateway.servicebus.windows.net");
    expect(within(sheet).getByLabelText("Name")).toHaveValue("Prod logistics");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(true));
    const draft = (mock.calls.find((c) => c.cmd === "profile_save")!.args as { draft: { auth: unknown } }).draft;
    expect(draft.auth).toEqual({ kind: "entra", namespace: "sb-partner-gateway.servicebus.windows.net", tenant: null });
  });

  it("notes in the summary when shared access keys are off", async () => {
    const { user, sheet } = await signedIn();
    await user.click(await within(sheet).findByRole("option", { name: /sb-finance-events/ }));
    expect(summary(sheet)).toHaveTextContent("Shared access keys are off for this namespace. Entra ID only.");
    await user.click(within(sheet).getByRole("button", { name: "Change" }));
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-dev/ }));
    expect(summary(sheet)).not.toHaveTextContent(/Shared access keys/);
  });

  it("selects with the keyboard: arrows move, Enter picks, and Enter does not save the form", async () => {
    const { user, sheet, mock } = await signedIn();
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    list.focus();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(list).toHaveAttribute("aria-activedescendant", within(list).getAllByRole("option")[2].id);
    await user.keyboard("{ArrowUp}{Enter}");
    expect(summary(sheet)).toHaveTextContent("sb-logistics-dev.servicebus.windows.net");
    expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(false);
    await user.click(within(sheet).getByRole("button", { name: "Change" }));
    const again = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    again.focus();
    await user.keyboard("{End}{ }");
    expect(summary(sheet)).toHaveTextContent("sb-partner-gateway.servicebus.windows.net");
    await user.click(within(sheet).getByRole("button", { name: "Change" }));
    (await within(sheet).findByRole("listbox", { name: "Namespaces" })).focus();
    await user.keyboard("{Home}{Enter}");
    expect(summary(sheet)).toHaveTextContent("sb-finance-events.servicebus.windows.net");
  });

  it("shows the keyboard position only as a row ring, not a ring around the whole list", async () => {
    const { sheet } = await signedIn();
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(list.className).toContain("outline-none");
    expect(list.className).not.toMatch(/focus-visible:outline/);
    expect(within(list).getAllByRole("option")[0].className).toContain("group-focus-visible:outline");
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

  it("offers to sign in to a tenant that needs multi-factor sign-in, then lists its namespaces, without resetting anything", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await user.type(within(sheet).getByLabelText("Name"), "Keep me");
    await chooseTenant(user, sheet, "Northwind Traders");
    const heading = await within(sheet).findByRole("heading", { name: "Northwind Traders needs its own sign-in" });
    // Not a failure: no amber error icon.
    expect(heading.querySelector("svg")).toBeNull();
    expect(within(sheet).getByText(/requires multi-factor sign-in/)).toBeInTheDocument();
    expect(within(sheet).queryByRole("listbox")).not.toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Sign in to Northwind Traders" }));
    expect(mock.calls.at(-1)).toMatchObject({ cmd: "sign_in", args: { tenant: "tenant-guest", deviceCode: false } });
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(within(list).getAllByRole("option").map(cells)).toEqual([
      ["nw-orders", "Northwind production", "westeurope", "Premium"],
      ["nw-telemetry", "Northwind shared", "uksouth", "Standard"],
    ]);
    // The picker did not remount: still on that tenant, the account line and the typed name are as they were.
    expect(within(sheet).getByRole("combobox", { name: "Tenant" })).toHaveTextContent("Northwind Traders");
    expect(within(sheet).getByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    expect(within(sheet).getByLabelText("Name")).toHaveValue("Keep me");
    // Picking one files it under that tenant, and the profile binds to that tenant's own sign-in.
    await user.click(within(list).getByRole("option", { name: /nw-orders/ }));
    expect(summary(sheet)).toHaveTextContent("nw-orders.servicebus.windows.net");
    expect(summary(sheet)).toHaveTextContent("Northwind Traders");
    await showManual(user, sheet);
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("tenant-guest");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(async () => {
      const saved = (await mock.invoke<Profile[]>("profiles_list")).find((p) => p.name === "Keep me");
      // Bound to that tenant's own sign-in, whose home is the tenant: no tenant is stored.
      expect(saved?.auth).toEqual({ kind: "entra", sign_in_id: "oid-guest", tenant: null });
      expect(saved?.bound_sign_in).toMatchObject({ tenant_id: "tenant-guest", live: true, same_account: true, own_tenant: true });
    });
  });

  it("says which identity a tenant is listed with, beside the tenant and in the summary", async () => {
    const { user, sheet } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await user.click(await within(sheet).findByRole("button", { name: "Sign in to Northwind Traders" }));
    expect(await within(sheet).findByText("Listed with your own sign-in to this tenant, as ops@contoso.example.")).toBeInTheDocument();
    await user.click(await within(sheet).findByRole("option", { name: /nw-orders/ }));
    expect(summary(sheet)).toHaveTextContent("Listed and connected with your own sign-in to Northwind Traders, as ops@contoso.example.");
  });

  it("edits a guest-tenant profile without calling it a different account, and offers its tenant's sign-in", async () => {
    const { user, sheet } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await user.click(await within(sheet).findByRole("button", { name: "Sign in to Northwind Traders" }));
    await user.click(await within(sheet).findByRole("option", { name: /nw-orders/ }));
    await user.type(within(sheet).getByLabelText("Name"), " NW");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.queryByRole("dialog", { name: "Add connection" })).not.toBeInTheDocument());
    // It connects with the tenant's own sign-in; wait for the workbench before using the menu.
    await within(await screen.findByRole("tree", { name: /^Namespace(,| \(|$)/ })).findByRole("treeitem", { name: /customs-docs/ });
    await chooseMenuItem(user, /Connection: nw-orders NW/, "Manage connections…");
    await user.click(within(await screen.findByRole("dialog", { name: "Connections" })).getByRole("button", { name: "Edit nw-orders NW" }));
    const edit = await screen.findByRole("dialog", { name: "Edit nw-orders NW" });
    expect(within(edit).getByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    expect(within(edit).queryByText(/different account|has expired/)).not.toBeInTheDocument();
    // Its tenant shows, though none is stored (the tenant's own sign-in is at home there).
    expect(within(edit).getByLabelText("Tenant")).toHaveValue("tenant-guest");
    expect(await within(edit).findByRole("button", { name: "Sign in to Northwind Traders" })).toBeInTheDocument();
  });

  it("rebinds an edited profile saved before its tenant sign-in, after 'Sign in to <tenant>'", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("tenant-sign-in");
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    await user.click(within(await screen.findByRole("dialog", { name: "Connections" })).getByRole("button", { name: "Edit Northwind orders" }));
    const edit = await screen.findByRole("dialog", { name: "Edit Northwind orders" });
    expect(within(edit).getByLabelText("Tenant")).toHaveValue("tenant-guest");
    await user.click(await within(edit).findByRole("button", { name: "Sign in to Northwind Traders" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "sign_in")).toBe(true));
    await within(edit).findByRole("button", { name: "Sign in to Northwind Traders" }); // finished
    await user.click(within(edit).getByRole("button", { name: "Save" }));
    await waitFor(async () => {
      const saved = (await mock.invoke<Profile[]>("profiles_list")).find((p) => p.id === "p-northwind");
      expect(saved?.auth).toEqual({ kind: "entra", sign_in_id: "oid-guest", tenant: null });
    });
  });

  it("clears a Tenant ID taken from the browsed tenant when the account changes", async () => {
    const { user, sheet } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await showManual(user, sheet);
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("tenant-guest");
    await user.click(within(sheet).getByRole("button", { name: "Use another account" }));
    await within(sheet).findByText(/Signed in as dev@fabrikam.example/);
    await waitFor(() => expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue(""));
  });

  it("keeps the home sign-in current after signing in to a tenant, so other profiles are not 'a different account'", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default");
    await mock.invoke("sign_in", { tenant: null, deviceCode: false });
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Add connection…" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("radio", { name: "Entra ID" }));
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await user.click(await within(sheet).findByRole("button", { name: "Sign in to Northwind Traders" }));
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(await mock.invoke("account_current")).toMatchObject({ id: "oid-1", tenant_id: "tenant-1" });
    await user.click(within(sheet).getByRole("button", { name: "Cancel" }));
    await user.click(await screen.findByRole("button", { name: /Connection: Choose a connection/ }));
    await user.click(await screen.findByRole("menuitem", { name: "Manage connections…" }));
    await user.click(within(await screen.findByRole("dialog", { name: "Connections" })).getByRole("button", { name: "Edit Logistics dev" }));
    const edit = await screen.findByRole("dialog", { name: "Edit Logistics dev" });
    expect(within(edit).getByText(/Signed in as ops@contoso.example/)).toBeInTheDocument();
    expect(within(edit).queryByText(/different account/)).not.toBeInTheDocument();
  });

  it("can sign in to the tenant with a device code, and cancelling returns to the message", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await user.click(await within(sheet).findByRole("button", { name: "Use a device code instead" }));
    expect(mock.calls.at(-1)).toMatchObject({ cmd: "sign_in", args: { tenant: "tenant-guest", deviceCode: true } });
    expect(await within(sheet).findByText("QX4T-9KLM")).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Cancel sign-in" }));
    expect(await within(sheet).findByRole("button", { name: "Sign in to Northwind Traders" })).toBeInTheDocument();
    expect(within(sheet).queryByText(/cancelled/i)).not.toBeInTheDocument();
  });

  it("changing the tenant drops a picked namespace and its tenant, but never typed text", async () => {
    const { user, sheet } = await signedIn();
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-prod/ }));
    await user.click(within(sheet).getByRole("button", { name: "Change" }));
    await chooseTenant(user, sheet, "Northwind Traders");
    expect(within(sheet).queryByRole("group", { name: "Selected namespace" })).not.toBeInTheDocument();
    await showManual(user, sheet);
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("");
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("tenant-guest"); // the browsed tenant, offered for typing
    // Typed text survives a tenant change; the Tenant ID, filled from the browsed tenant, follows it home.
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-typed");
    await chooseTenant(user, sheet, "Horizon Tech (home)");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-typed");
    expect(within(sheet).getByLabelText("Tenant ID")).toHaveValue("");
  });

  it("keeps the Tenant ID on the browsed tenant until it is typed in, then leaves it alone", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    await chooseTenant(user, sheet, "Northwind Traders");
    await showManual(user, sheet);
    const tenantId = () => within(sheet).getByLabelText("Tenant ID");
    expect(tenantId()).toHaveValue("tenant-guest");
    await chooseTenant(user, sheet, "Horizon Tech (home)");
    expect(tenantId()).toHaveValue("");
    await chooseTenant(user, sheet, "Northwind Traders");
    expect(tenantId()).toHaveValue("tenant-guest");
    // Typed: the select no longer moves it.
    await user.clear(tenantId());
    await user.type(tenantId(), "tenant-typed");
    await chooseTenant(user, sheet, "Horizon Tech (home)");
    expect(tenantId()).toHaveValue("tenant-typed");
    await user.type(within(sheet).getByLabelText("Name"), "Typed");
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-typed");
    await user.click(within(sheet).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mock.calls.some((c) => c.cmd === "profile_save")).toBe(true));
    const draft = (mock.calls.find((c) => c.cmd === "profile_save")!.args as { draft: { auth: unknown } }).draft;
    expect(draft.auth).toEqual({ kind: "entra", namespace: "sb-typed", tenant: "tenant-typed" });
  });

  it("moves focus to Change after a pick, and back to the picked row after Change", async () => {
    const { user, sheet } = await signedIn();
    // A click picks: focus lands on Change, since the list it was in is gone.
    await user.click(await within(sheet).findByRole("option", { name: /sb-logistics-dev/ }));
    expect(within(summary(sheet)).getByRole("button", { name: "Change" })).toHaveFocus();
    // Change: the list has focus, its keyboard position on the picked row.
    await user.keyboard("{Enter}");
    const list = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(list).toHaveFocus();
    const picked = within(list).getByRole("option", { name: /sb-logistics-dev/ });
    expect(list).toHaveAttribute("aria-activedescendant", picked.id);
    // Enter picks from the keyboard and focus is on Change again.
    await user.keyboard("{ArrowDown}{Enter}");
    expect(summary(sheet)).toHaveTextContent("sb-logistics-prod.servicebus.windows.net");
    expect(within(summary(sheet)).getByRole("button", { name: "Change" })).toHaveFocus();
    await user.click(within(summary(sheet)).getByRole("button", { name: "Change" }));
    const again = await within(sheet).findByRole("listbox", { name: "Namespaces" });
    expect(again).toHaveFocus();
    expect(again).toHaveAttribute("aria-activedescendant", within(again).getByRole("option", { name: /sb-logistics-prod/ }).id);
  });

  it("selects the sign-in's own tenant, or 'Home tenant', when the list does not mark a home tenant", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("empty");
    const invoke = mock.invoke.bind(mock);
    let tenants = [
      { tenant_id: "tenant-guest", display_name: "Northwind Traders", default_domain: null, home: false },
      { tenant_id: "tenant-1", display_name: "Horizon Tech", default_domain: null, home: false },
    ];
    mock.invoke = (async (cmd: string, args?: Record<string, unknown>) => (cmd === "list_tenants" ? tenants : invoke(cmd, args))) as typeof mock.invoke;
    await user.click(await screen.findByRole("button", { name: "Sign in with Entra ID" }));
    const sheet = await screen.findByRole("dialog", { name: "Add connection" });
    await user.click(within(sheet).getByRole("button", { name: "Sign in with Entra ID" }));
    // The account's own tenant (tenant-1) is the home one, and is labelled as such.
    expect(await within(sheet).findByRole("combobox", { name: "Tenant" })).toHaveTextContent("Horizon Tech (home)");
    await user.click(within(sheet).getByRole("button", { name: "Use another account" }));
    // A list that names neither: "Home tenant" is the first option and is selected.
    tenants = [{ tenant_id: "tenant-guest", display_name: "Northwind Traders", default_domain: null, home: false }];
    await within(sheet).findByText(/Signed in as dev@fabrikam.example/);
    const select = await within(sheet).findByRole("combobox", { name: "Tenant" });
    await waitFor(() => expect(select).toHaveTextContent("Home tenant"));
    await user.click(select);
    expect((await screen.findAllByRole("option")).map((o) => o.textContent)).toEqual(["Home tenant", "Northwind Traders"]);
  });

  it("keeps manual entry one click away, as a toggle that can hide the fields again", async () => {
    const { user, sheet, mock } = await signedIn();
    await within(sheet).findByRole("listbox", { name: "Namespaces" });
    const toggle = within(sheet).getByRole("button", { name: "Enter a namespace manually" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(toggle.querySelector("svg")).not.toHaveClass("rotate-90");
    await user.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(toggle.querySelector("svg")).toHaveClass("rotate-90");
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("");
    await user.type(within(sheet).getByLabelText("Name"), "Typed");
    await user.type(within(sheet).getByLabelText("Namespace"), "sb-typed");
    await user.click(toggle);
    expect(within(sheet).queryByLabelText("Namespace")).not.toBeInTheDocument();
    await user.click(toggle);
    expect(within(sheet).getByLabelText("Namespace")).toHaveValue("sb-typed"); // hiding does not lose it
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
    expect(await within(sheet).findByText("No Service Bus namespaces in this tenant")).toBeInTheDocument();
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
    expect(await within(sheet).findByRole("status", { name: "Loading namespaces" })).toBeInTheDocument();
    release();
    expect(await within(sheet).findByRole("listbox", { name: "Namespaces" })).toBeInTheDocument();
    expect(within(sheet).queryByRole("status", { name: "Loading namespaces" })).not.toBeInTheDocument();
  });

  it("names the subscriptions that could not be listed, with the reasons one click away", async () => {
    const { user, sheet } = await signedIn();
    const summaryEl = (await within(sheet).findByText("1 subscription couldn't be listed")).closest("summary")!;
    expect(summaryEl.querySelector("svg")).not.toHaveClass("rotate-90"); // the chevron, not the native marker
    expect(summaryEl.className).toContain("list-none");
    expect(within(sheet).getByText(/you need Reader on this subscription/i)).not.toBeVisible(); // closed until asked for
    await user.click(summaryEl);
    expect(within(sheet).getByText(/you need Reader on this subscription/i)).toBeVisible();
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
