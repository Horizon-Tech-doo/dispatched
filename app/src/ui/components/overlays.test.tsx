import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "./tabs";
import { Sheet } from "./sheet";
import { CommandGroup, CommandInput, CommandItem, CommandList, CommandPaletteDialog } from "./command";
import { DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "./dropdown-menu";
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
    // Clear of the 28px fade on the edge with more tabs beyond it: the tab's right edge (200) sits 28px inside the strip's (100).
    await vi.waitFor(() => expect(list.scrollLeft).toBe(128));
    await userEvent.click(screen.getByRole("tab", { name: "orders" }));
    await vi.waitFor(() => expect(list.scrollLeft).toBe(0));
    expect(scrollIntoView).not.toHaveBeenCalled();
    spy.mockRestore();
  });
});

describe("Tabs that do not fit", () => {
  let sizes = { scrollWidth: 300, clientWidth: 100 };
  const watchers: (() => void)[] = [];
  const stubLayout = () => {
    vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(() => sizes.scrollWidth);
    vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => sizes.clientWidth);
    vi.stubGlobal(
      "ResizeObserver",
      class {
        constructor(cb: () => void) {
          watchers.push(cb);
        }
        observe() {}
        unobserve() {}
        disconnect() {}
      },
    );
  };
  afterEach(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    watchers.length = 0;
    sizes = { scrollWidth: 300, clientWidth: 100 };
  });
  const strip = () => (
    <Tabs defaultValue="a">
      <TabsList variant="underline" aria-label="Views">
        <TabsTrigger value="a">Overview</TabsTrigger>
        <TabsTrigger value="b">Scheduled</TabsTrigger>
      </TabsList>
    </Tabs>
  );

  it("marks the edge that has more tabs beyond it, and updates it as the strip scrolls", () => {
    stubLayout();
    render(strip());
    const list = screen.getByRole("tablist");
    expect(list).not.toHaveAttribute("data-overflow-start");
    expect(list).toHaveAttribute("data-overflow-end");
    list.scrollLeft = 80;
    fireEvent.scroll(list);
    expect(list).toHaveAttribute("data-overflow-start");
    expect(list).toHaveAttribute("data-overflow-end");
    list.scrollLeft = 200; // the far end: scrollWidth - clientWidth
    fireEvent.scroll(list);
    expect(list).toHaveAttribute("data-overflow-start");
    expect(list).not.toHaveAttribute("data-overflow-end");
  });

  it("marks nothing when the tabs fit", () => {
    sizes = { scrollWidth: 100, clientWidth: 100 };
    stubLayout();
    render(strip());
    const list = screen.getByRole("tablist");
    expect(list).not.toHaveAttribute("data-overflow-start");
    expect(list).not.toHaveAttribute("data-overflow-end");
  });

  it("opens at the far end when the last tab is active, padding included, so no fade claims there is more", () => {
    stubLayout();
    const rect = (l: number, w: number) => ({ left: l, right: l + w, top: 0, bottom: 30, width: w, height: 30, x: l, y: 0, toJSON() {} }) as DOMRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.getAttribute("role") === "tablist") return rect(0, 100);
      const list = this.closest<HTMLElement>('[role="tablist"]')!;
      return rect((this.textContent === "Scheduled" ? 240 : 0) - list.scrollLeft, 50);
    });
    render(
      <Tabs defaultValue="b">
        <TabsList variant="underline" aria-label="Views">
          <TabsTrigger value="a">Overview</TabsTrigger>
          <TabsTrigger value="b">Scheduled</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    const list = screen.getByRole("tablist");
    expect(list.scrollLeft).toBe(200); // scrollWidth - clientWidth, past the tab's own edge (190) by the padding
    expect(list).toHaveAttribute("data-overflow-start");
    expect(list).not.toHaveAttribute("data-overflow-end");
  });

  it("reveals a middle tab clear of the fades on both edges", async () => {
    sizes = { scrollWidth: 600, clientWidth: 100 };
    stubLayout();
    const rect = (l: number, w: number) => ({ left: l, right: l + w, top: 0, bottom: 30, width: w, height: 30, x: l, y: 0, toJSON() {} }) as DOMRect;
    // Four tabs of 50px at 0, 150, 300 and 450 in the strip's own coordinates (the strip shows 100px).
    const at: Record<string, number> = { one: 0, two: 150, three: 300, four: 450 };
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.getAttribute("role") === "tablist") return rect(0, 100);
      const list = this.closest<HTMLElement>('[role="tablist"]')!;
      return rect(at[this.textContent ?? ""] - list.scrollLeft, 50);
    });
    render(
      <Tabs defaultValue="one">
        <TabsList variant="underline" aria-label="Views">
          {Object.keys(at).map((n) => (
            <TabsTrigger key={n} value={n}>{n}</TabsTrigger>
          ))}
        </TabsList>
      </Tabs>,
    );
    const list = screen.getByRole("tablist");
    expect(list.scrollLeft).toBe(0);
    await userEvent.click(screen.getByRole("tab", { name: "three" }));
    // Coming from the left: the tab's right edge (350) ends 28px inside the strip's right edge.
    await vi.waitFor(() => expect(list.scrollLeft).toBe(350 - 72));
    const inside = (name: string) => {
      const t = at[name] - list.scrollLeft;
      return { fromLeft: t, fromRight: 100 - (t + 50) };
    };
    expect(inside("three").fromRight).toBeGreaterThanOrEqual(28);
    await userEvent.click(screen.getByRole("tab", { name: "two" }));
    // Coming from the right: the tab's left edge (150) ends 28px inside the strip's left edge.
    await vi.waitFor(() => expect(list.scrollLeft).toBe(150 - 28));
    expect(inside("two").fromLeft).toBeGreaterThanOrEqual(28);
  });

  it("watches tabs that are added later, and the strip has no gap that the fades would share", async () => {
    stubLayout();
    const watched = new Set<Element>();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe(el: Element) {
          watched.add(el);
        }
        unobserve() {}
        disconnect() {}
      },
    );
    const view = (tabs: string[]) => (
      <Tabs defaultValue="a">
        <TabsList variant="underline" aria-label="Views">
          {tabs.map((t) => (
            <TabsTrigger key={t} value={t}>{t}</TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
    );
    const { rerender } = render(view(["a", "b"]));
    expect(watched.has(screen.getByRole("tab", { name: "b" }))).toBe(true);
    rerender(view(["a", "b", "c"]));
    await vi.waitFor(() => expect(watched.has(screen.getByRole("tab", { name: "c" }))).toBe(true));
    // The pseudo-element ends take part in a flex gap like any item; the space between tabs is margin instead.
    expect(screen.getByRole("tablist").className).not.toMatch(/(^|\s)gap-/);
    expect(screen.getByRole("tab", { name: "b" }).className).toContain("not-first:ml-0.5");
  });

  it("keeps the active tab in view when the strip resizes, until the person scrolls it themselves", () => {
    sizes = { scrollWidth: 200, clientWidth: 100 };
    stubLayout();
    let left = 0;
    const rect = (l: number, w: number) => ({ left: l, right: l + w, top: 0, bottom: 30, width: w, height: 30, x: l, y: 0, toJSON() {} }) as DOMRect;
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
      if (this.getAttribute("role") === "tablist") return rect(0, 100);
      const list = this.closest<HTMLElement>('[role="tablist"]')!;
      return rect((this.textContent === "Scheduled" ? 150 : 0) - list.scrollLeft, 50);
    });
    render(
      <Tabs defaultValue="b">
        <TabsList variant="underline" aria-label="Views">
          <TabsTrigger value="a">Overview</TabsTrigger>
          <TabsTrigger value="b">Scheduled</TabsTrigger>
        </TabsList>
      </Tabs>,
    );
    const list = screen.getByRole("tablist");
    // On mount the active tab (right edge at 200) is brought into the 100px strip.
    expect(list.scrollLeft).toBe(100);
    list.scrollLeft = 0;
    left = list.scrollLeft;
    watchers.forEach((w) => w()); // a count arrived and a tab grew
    expect(list.scrollLeft).toBe(100);
    // The person scrolls back; the next resize leaves that alone.
    fireEvent.wheel(list, { deltaY: -100 });
    list.scrollLeft = left;
    watchers.forEach((w) => w());
    expect(list.scrollLeft).toBe(0);
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
  it("checkbox items report their state and toggle", async () => {
    const onCheckedChange = vi.fn();
    render(
      <DropdownMenu>
        <DropdownMenuTrigger asChild><Button>Columns</Button></DropdownMenuTrigger>
        <DropdownMenuContent>
          <DropdownMenuCheckboxItem checked onCheckedChange={onCheckedChange}>Subject</DropdownMenuCheckboxItem>
          <DropdownMenuCheckboxItem checked={false}>Reason</DropdownMenuCheckboxItem>
        </DropdownMenuContent>
      </DropdownMenu>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Columns" }));
    expect(await screen.findByRole("menuitemcheckbox", { name: "Subject" })).toHaveAttribute("aria-checked", "true");
    expect(screen.getByRole("menuitemcheckbox", { name: "Reason" })).toHaveAttribute("aria-checked", "false");
    await userEvent.click(screen.getByRole("menuitemcheckbox", { name: "Subject" }));
    expect(onCheckedChange).toHaveBeenCalledWith(false);
  });
});
