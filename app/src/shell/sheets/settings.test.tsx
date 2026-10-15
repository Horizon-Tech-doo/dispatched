import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpdater } from "@/engine";
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

describe("update prompt", () => {
  it("announces an update in the status bar and installs it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const install = vi.fn(async (cb: (p: number | null) => void) => cb(100));
    setUpdater({ check: async () => ({ version: "0.1.0", notes: "Faster tree loading.", date: "2026-10-20" }), install });
    renderShell("default");
    vi.advanceTimersByTime(3500);
    const notice = await screen.findByRole("button", { name: "Update 0.1.0 available" });
    vi.useRealTimers();
    const user = userEvent.setup();
    await user.click(notice);
    const sheet = await screen.findByRole("dialog", { name: "Update available" });
    expect(within(sheet).getByText("Faster tree loading.")).toBeInTheDocument();
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    expect(install).toHaveBeenCalledOnce();
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
    const sheet = await screen.findByRole("dialog", { name: "Dispatched is up to date" });
    expect(within(sheet).getByText("You have the latest version.")).toBeInTheDocument();
    expect(check).toHaveBeenCalled();
    const before = check.mock.calls.length;
    await user.click(within(sheet).getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(check.mock.calls.length).toBeGreaterThan(before));
  });

  it("keeps the sheet open and shows the reason when the install fails", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    setUpdater({
      check: async () => ({ version: "0.1.0", notes: null, date: null }),
      install: async () => {
        throw new Error("Download failed");
      },
    });
    renderShell("default");
    vi.advanceTimersByTime(3500);
    const notice = await screen.findByRole("button", { name: "Update 0.1.0 available" });
    vi.useRealTimers();
    const user = userEvent.setup();
    await user.click(notice);
    const sheet = await screen.findByRole("dialog", { name: "Update available" });
    await user.click(within(sheet).getByRole("button", { name: "Install and restart" }));
    expect(await within(sheet).findByText("Download failed")).toBeInTheDocument();
    expect(within(sheet).getByRole("button", { name: "Install and restart" })).toBeEnabled();
  });
});
