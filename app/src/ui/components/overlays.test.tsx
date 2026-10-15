import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
import { Sheet } from "./sheet";
import { CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog } from "./command";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./dropdown-menu";
import { Button } from "./button";

describe("Tabs", () => {
  it("switch with arrow keys", async () => {
    render(
      <Tabs defaultValue="a">
        <TabsList variant="underline" aria-label="Views">
          <TabsTrigger value="a">Overview</TabsTrigger>
          <TabsTrigger value="b">Dead-letter</TabsTrigger>
        </TabsList>
        <TabsContent value="a">A body</TabsContent>
        <TabsContent value="b">B body</TabsContent>
      </Tabs>,
    );
    await userEvent.click(screen.getByRole("tab", { name: "Overview" }));
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Dead-letter" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByText("B body")).toBeVisible();
  });
});

describe("Tabs overflow", () => {
  it("maps the vertical wheel to horizontal scrolling on a strip", () => {
    render(
      <Tabs defaultValue="a">
        <TabsList variant="strip" aria-label="Open entities">
          <TabsTrigger value="a">orders</TabsTrigger>
          <TabsTrigger value="b">audit</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    const list = screen.getByRole("tablist");
    fireEvent.wheel(list, { deltaY: 40 });
    expect(list.scrollLeft).toBe(40);
  });

  it("reveals the active tab by scrolling the strip only, never the window", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    const rect = (left: number, width: number) => ({ left, right: left + width, top: 0, bottom: 30, width, height: 30, x: left, y: 0, toJSON() {} });
    const spy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.getAttribute("role") === "tablist") return rect(0, 100) as DOMRect;
      // The tabs sit where they would after scrolling by the list's scrollLeft.
      const list = this.closest<HTMLElement>('[role="tablist"]')!;
      return rect((this.textContent === "audit" ? 150 : 0) - list.scrollLeft, 50) as DOMRect;
    });
    render(
      <Tabs defaultValue="a">
        <TabsList variant="strip" aria-label="Open entities">
          <TabsTrigger value="a">orders</TabsTrigger>
          <TabsTrigger value="b">audit</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    const list = screen.getByRole("tablist");
    expect(list.scrollLeft).toBe(0);
    await userEvent.click(screen.getByRole("tab", { name: "audit" }));
    await vi.waitFor(() => expect(list.scrollLeft).toBe(100));
    await userEvent.click(screen.getByRole("tab", { name: "orders" }));
    await vi.waitFor(() => expect(list.scrollLeft).toBe(0));
    expect(scrollIntoView).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

describe("Tabs focus", () => {
  it("marks a tab pressed with the pointer so it shows its marker, not the keyboard ring", async () => {
    render(
      <Tabs defaultValue="a">
        <TabsList aria-label="Drawer">
          <TabsTrigger value="a">Jobs</TabsTrigger>
          <TabsTrigger value="b">Activity</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    const jobs = screen.getByRole("tab", { name: "Jobs" });
    expect(jobs.className).toContain("focus-visible:-outline-offset-2");
    expect(jobs.className).toContain("data-[pointer]:focus-visible:outline-0");
    fireEvent.mouseDown(jobs, { button: 0 });
    expect(jobs).toHaveAttribute("data-pointer");
    expect(jobs).toHaveFocus();
    // The next key brings the ring back.
    fireEvent.keyDown(jobs, { key: "ArrowRight" });
    expect(jobs).not.toHaveAttribute("data-pointer");
    fireEvent.mouseDown(jobs, { button: 0 });
    fireEvent.blur(jobs);
    expect(jobs).not.toHaveAttribute("data-pointer");
  });
});

describe("Sheet", () => {
  it("opens without a description and without warnings", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    render(<Sheet open onOpenChange={() => {}} title="Filters"><span>Body</span></Sheet>);
    expect(screen.getByRole("dialog", { name: "Filters" })).toBeInTheDocument();
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
    warn.mockRestore();
    error.mockRestore();
  });

  it("is a labelled dialog that closes on Escape", async () => {
    const onOpenChange = vi.fn();
    render(
      <Sheet open onOpenChange={onOpenChange} title="Add connection" description="Saved on this computer.">
        <input aria-label="Name" />
      </Sheet>,
    );
    expect(screen.getByRole("dialog", { name: "Add connection" })).toHaveAccessibleDescription("Saved on this computer.");
    await userEvent.keyboard("{Escape}");
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});

describe("Command palette", () => {
  it("filters and runs an item", async () => {
    const onSelect = vi.fn();
    render(
      <CommandPaletteDialog open onOpenChange={() => {}}>
        <CommandInput placeholder="Type a command" />
        <CommandList>
          <CommandGroup heading="App">
            <CommandItem onSelect={onSelect}>Toggle theme</CommandItem>
            <CommandItem onSelect={() => {}}>Refresh</CommandItem>
          </CommandGroup>
        </CommandList>
      </CommandPaletteDialog>,
    );
    await userEvent.type(screen.getByPlaceholderText("Type a command"), "theme");
    expect(screen.queryByText("Refresh")).not.toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(onSelect).toHaveBeenCalledOnce();
  });
});

describe("DropdownMenu", () => {
  it("opens from its trigger and selects an item", async () => {
    const onSelect = vi.fn();
    render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild><Button>Logistics</Button></DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuItem onSelect={onSelect}>Manage profiles</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Logistics" }));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Manage profiles" }));
    expect(onSelect).toHaveBeenCalledOnce();
  });
});
