import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { Button } from "./button";
import { IconButton } from "./icon-button";
import { Field } from "./field";
import { Input } from "./input";
import { CountBadge, ProdBadge } from "./badge";
import { Kbd } from "./kbd";
import { EmptyState } from "./empty-state";
import { SkeletonRows } from "./skeleton";
import { ToggleGroup } from "./toggle-group";

describe("Button", () => {
  it("is a type=button that clicks and disables", async () => {
    const onClick = vi.fn();
    const { rerender } = render(<Button onClick={onClick}>Refresh</Button>);
    const b = screen.getByRole("button", { name: "Refresh" });
    expect(b).toHaveAttribute("type", "button");
    await userEvent.click(b);
    expect(onClick).toHaveBeenCalledOnce();
    rerender(<Button onClick={onClick} disabled>Refresh</Button>);
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("never fills a variant with the accent (amber is reserved for dead letters, PROD and errors)", () => {
    const variants = ["default", "quiet", "primary", "danger"] as const;
    render(<>{variants.map((v) => <Button key={v} variant={v}>{v}</Button>)}</>);
    for (const v of variants) {
      expect(screen.getByRole("button", { name: v }).className).not.toContain("bg-accent");
    }
  });
});

describe("IconButton", () => {
  it("names the button with its label", () => {
    render(<IconButton label="Refresh all"><svg /></IconButton>);
    expect(screen.getByRole("button", { name: "Refresh all" })).toBeInTheDocument();
  });
});

describe("Field", () => {
  it("wires label, hint and error to the control", () => {
    render(
      <Field label="Name" hint="Shown in the connection list" error="A profile named Logistics already exists">
        {(ids) => <Input {...ids} defaultValue="Logistics" />}
      </Field>,
    );
    const input = screen.getByLabelText("Name");
    expect(input).toHaveAttribute("aria-invalid", "true");
    expect(input).toHaveAccessibleDescription(/already exists/);
    expect(input).toHaveAccessibleDescription(/Shown in the connection list/);
  });
});

describe("CountBadge", () => {
  it("formats and marks attention only above zero", () => {
    const { rerender } = render(<CountBadge value={3417} attention />);
    expect(screen.getByText("3,417").className).toContain("text-accent");
    rerender(<CountBadge value={0} attention />);
    expect(screen.getByText("0").className).not.toContain("text-accent");
    rerender(<CountBadge value={null} attention />);
    expect(screen.getByText("?")).toBeInTheDocument();
    rerender(<CountBadge value={120} quality={{ approximate: true }} />);
    expect(screen.getByText("~120")).toBeInTheDocument();
  });
});

describe("ProdBadge, Kbd, EmptyState, SkeletonRows", () => {
  it("render accessibly", () => {
    render(
      <>
        <ProdBadge />
        <Kbd shortcut="mod+k" />
        <EmptyState title="No dead letters" body="Messages that fail delivery land here." tone="neutral" />
        <SkeletonRows rows={3} />
      </>,
    );
    expect(screen.getByText("PROD")).toHaveAttribute("title", "Production profile");
    expect(screen.getByText(/K$/)).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "No dead letters" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "Loading" }).querySelectorAll("[data-skeleton-row]")).toHaveLength(3);
  });
});

describe("ToggleGroup", () => {
  it("toggles values independently with aria-pressed", async () => {
    const onChange = vi.fn();
    const opts = [{ value: "active", label: "Active" }, { value: "dlq", label: "Dead-letter" }] as const;
    render(<ToggleGroup label="Counts" values={["active"]} options={[...opts]} onChange={onChange} />);
    expect(screen.getByRole("group", { name: "Counts" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Active" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Dead-letter" })).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(screen.getByRole("button", { name: "Dead-letter" }));
    expect(onChange).toHaveBeenCalledWith(["active", "dlq"]);
    await userEvent.click(screen.getByRole("button", { name: "Active" }));
    expect(onChange).toHaveBeenLastCalledWith([]);
  });
  it("separates the buttons and gives on and off items their hover states", () => {
    render(<ToggleGroup label="Counts" values={["a"]} options={[{ value: "a", label: "A" }, { value: "b", label: "B" }]} onChange={() => {}} />);
    expect(screen.getByRole("group", { name: "Counts" }).className).toContain("gap-px");
    expect(screen.getByRole("button", { name: "B" }).className).toContain("hover:bg-hover");
    expect(screen.getByRole("button", { name: "A" }).className).toContain("hover:bg-selection");
  });
});
