import { act, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpdater, type UpdateInfo } from "@/engine";
import { jobStore } from "../drawer/activity";
import { renderShell } from "../test-utils";

afterEach(() => vi.useRealTimers());

describe("SettingsSheet", () => {
  it("changes theme and density and saves them", async () => {
    const user = userEvent.setup();
    const { mock } = renderShell("default");
    await screen.findByRole("list", { name: "Saved connections" });
    await user.keyboard("{Control>},{/Control}");
    const sheet = await screen.findByRole("dialog", { name: "Settings" });
    await user.click(within(sheet).getByRole("radio", { name: "Light" }));
    await waitFor(() => expect(document.documentElement.dataset.theme).toBe("light"));
    await user.click(within(sheet).getByRole("radio", { name: "Comfortable" }));
    await waitFor(() => expect(document.documentElement.style.getPropertyValue("--row")).toBe("26px"));
    await user.click(within(sheet).getByRole("radio", { name: "60 s" }));
    expect(mock.calls).toContainEqual({ cmd: "settings_set", args: { key: "ui", value: { theme: "light", density: "comfortable", autoRefresh: 60 } } });
    expect(await within(sheet).findByText("0.0.1-mock")).toBeInTheDocument();
  });
});

const INFO: UpdateInfo = { version: "0.1.0", notes: "Faster tree loading.", date: "2026-10-20" };
const NEVER = () => new Promise<void>(() => {});

/** Renders the shell with fake timers, waits for the status bar notice, and opens the update sheet from it. */
async function openUpdateSheet() {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  renderShell("default");
  vi.advanceTimersByTime(3500);
  const notice = await screen.findByRole("button", { name: "Update 0.1.0 available" });
  vi.useRealTimers();
  const user = userEvent.setup();
  await user.click(notice);
  return { user, sheet: await screen.findByRole("dialog", { name: "Updates" }) };
}

describe("update prompt", () => {
  it("announces an update in the status bar and installs it", async () => {
    const install = vi.fn(async (cb: (p: number | null) => void) => cb(100));
    setUpdater({ check: async () => INFO, install });
    const { user, sheet } = await openUpdateSheet();
    expect(await within(sheet).findByText("Faster tree loading.")).toBeInTheDocument();
    expect(within(sheet).getByText("You have 0.0.1-mock.")).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    expect(install).toHaveBeenCalledOnce();
  });

  it("checks exactly once 3 s after start when no sheet is open", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const check = vi.fn(async () => INFO);
    setUpdater({ check, install: async () => {} });
    renderShell("default");
    expect(check).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(3500);
    await screen.findByRole("button", { name: "Update 0.1.0 available" });
    await act(() => vi.advanceTimersByTimeAsync(10_000));
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("does not check twice when the sheet is opened before the start-up check", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const check = vi.fn(async () => null);
    setUpdater({ check, install: async () => {} });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderShell("default");
    await screen.findByRole("list", { name: "Saved connections" });
    await user.keyboard("{Control>},{/Control}");
    await user.click(await screen.findByRole("button", { name: "Check for updates" }));
    await screen.findByText(/^Dispatched 0\.0\.1-mock\./);
    await act(() => vi.advanceTimersByTimeAsync(5000)); // the start-up timer fires; the result is still fresh
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("says so when there is nothing to install, and checks again on demand", async () => {
    const user = userEvent.setup();
    const check = vi.fn(async () => null);
    setUpdater({ check, install: async () => {} });
    renderShell("default");
    await screen.findByRole("list", { name: "Saved connections" });
    await user.keyboard("{Control>},{/Control}");
    const settings = await screen.findByRole("dialog", { name: "Settings" });
    await user.click(within(settings).getByRole("button", { name: "Check for updates" }));
    const sheet = await screen.findByRole("dialog", { name: "Updates" });
    expect(await within(sheet).findByText(/^Dispatched 0\.0\.1-mock\. Checked \d\d:\d\d:\d\d\.$/)).toBeInTheDocument();
    const before = check.mock.calls.length;
    await user.click(within(sheet).getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(check.mock.calls.length).toBeGreaterThan(before));
  });

  it("shows the download as a progress bar and cannot be closed or replaced meanwhile", async () => {
    const install = vi.fn(async (cb: (p: number | null) => void) => {
      cb(60);
      await NEVER();
    });
    setUpdater({ check: async () => INFO, install });
    const { user, sheet } = await openUpdateSheet();
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    const bar = await within(sheet).findByRole("progressbar");
    expect(bar).toHaveAttribute("aria-valuenow", "60");
    expect(bar).toHaveAttribute("aria-valuemin", "0");
    expect(bar).toHaveAttribute("aria-valuemax", "100");
    expect(within(sheet).getByRole("status")).toHaveTextContent("Downloading 60%");
    expect(within(sheet).getByRole("button", { name: "Downloading…" })).toBeDisabled();
    expect(within(sheet).getByRole("button", { name: "Later" })).toBeDisabled();
    expect(within(sheet).queryByRole("button", { name: "Close" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    await user.keyboard("{Control>},{/Control}");
    expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument();
    expect(screen.getByRole("dialog", { name: "Updates" })).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "60");
  });

  it("has an indeterminate bar while the size is unknown and then says it is installing", async () => {
    let finish: () => void = () => {};
    setUpdater({
      check: async () => INFO,
      install: async (cb, onInstalling) => {
        cb(null);
        await new Promise<void>((r) => (finish = r));
        onInstalling?.();
        await NEVER();
      },
    });
    const { user, sheet } = await openUpdateSheet();
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    const bar = await within(sheet).findByRole("progressbar");
    expect(bar).not.toHaveAttribute("aria-valuenow");
    finish();
    await waitFor(() => expect(within(sheet).getByRole("status")).toHaveTextContent("Installing…"));
    expect(within(sheet).queryByRole("progressbar")).not.toBeInTheDocument();
  });

  it("keeps the sheet open, names the problem and offers Try again when the install fails", async () => {
    // The updater plugin rejects with plain strings.
    setUpdater({ check: async () => INFO, install: () => Promise.reject("Download failed") });
    const { user, sheet } = await openUpdateSheet();
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    expect(await within(sheet).findByText("Couldn't install the update")).toBeInTheDocument();
    expect(within(sheet).getByText("Download failed")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Try again" })).toBeEnabled();
    expect(within(sheet).getByRole("button", { name: "Close" })).toBeInTheDocument();
  });

  it("shows a failed check with its reason", async () => {
    setUpdater({ check: () => Promise.reject("Could not reach the update server."), install: async () => {} });
    const user = userEvent.setup();
    renderShell("default");
    await screen.findByRole("list", { name: "Saved connections" });
    await user.keyboard("{Control>},{/Control}");
    await user.click(await screen.findByRole("button", { name: "Check for updates" }));
    const sheet = await screen.findByRole("dialog", { name: "Updates" });
    expect(await within(sheet).findByText("Couldn't check for updates")).toBeInTheDocument();
    expect(within(sheet).getByText("Could not reach the update server.")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Check again" })).toBeEnabled();
  });

  it("still shows a failed re-check while an update is already known", async () => {
    let calls = 0;
    setUpdater({
      check: async () => {
        if (++calls > 1) throw "Offline";
        return INFO;
      },
      install: async () => {},
    });
    const { sheet } = await openUpdateSheet();
    expect(await within(sheet).findByText("Couldn't check for updates")).toBeInTheDocument();
    expect(within(sheet).getByText("Offline")).toBeInTheDocument();
    expect(within(sheet).getByText("Version 0.1.0", { exact: false })).toBeInTheDocument();
  });

  it("warns that running jobs stop on restart", async () => {
    setUpdater({ check: async () => INFO, install: async () => {} });
    const { sheet } = await openUpdateSheet();
    expect(within(sheet).queryByText(/running will stop/)).not.toBeInTheDocument();
    jobStore.apply({ type: "started", id: "j1", title: "Export", total: 10 });
    expect(await within(sheet).findByText("1 job running will stop when Dispatched restarts.")).toBeInTheDocument();
    jobStore.apply({ type: "started", id: "j2", title: "Purge", total: null });
    expect(await within(sheet).findByText("2 jobs running will stop when Dispatched restarts.")).toBeInTheDocument();
  });
});
