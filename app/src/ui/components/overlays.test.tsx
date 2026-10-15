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

  it("reveals the active tab when the selection changes", async () => {
    const scrollIntoView = vi.fn();
    Element.prototype.scrollIntoView = scrollIntoView;
    render(
      <Tabs defaultValue="a">
        <TabsList variant="strip" aria-label="Open entities">
          <TabsTrigger value="a">orders</TabsTrigger>
          <TabsTrigger value="b">audit</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    scrollIntoView.mockClear();
    await userEvent.click(screen.getByRole("tab", { name: "audit" }));
    await vi.waitFor(() => expect(scrollIntoView).toHaveBeenCalledWith({ inline: "nearest", block: "nearest" }));
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
