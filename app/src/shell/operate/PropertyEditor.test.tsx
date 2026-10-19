import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { TooltipProvider } from "@/ui";
import { PropertyEditor, propertyErrors, type PropRow } from "./PropertyEditor";
import { checkValue, incrementId } from "./props";

function Editor({ initial = [] as PropRow[] }) {
  const [rows, setRows] = useState<PropRow[]>(initial);
  return (
    <TooltipProvider>
      <PropertyEditor rows={rows} onChange={setRows} />
    </TooltipProvider>
  );
}

describe("PropertyEditor", () => {
  it("int out of range is an invalid field", async () => {
    render(<Editor />);
    await userEvent.click(screen.getByRole("button", { name: "Add property" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Property 1 name" }), "attempt");
    await userEvent.click(screen.getByRole("combobox", { name: "Property 1 type" }));
    await userEvent.click(await screen.findByRole("option", { name: "int" }));
    const value = screen.getByRole("textbox", { name: "Property 1 value" });
    await userEvent.type(value, "3000000000");
    expect(value).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("3000000000 does not fit an int (32 bits). Use long.")).toBeInTheDocument();
    await userEvent.clear(value);
    await userEvent.type(value, "-2147483648");
    expect(value).not.toHaveAttribute("aria-invalid");
  });

  it("a name set twice is an invalid field", () => {
    const rows: PropRow[] = [
      { key: 1, name: "a", type: "string", value: "x" },
      { key: 2, name: "a", type: "int", value: "1" },
    ];
    expect(propertyErrors(rows).get(2)).toBe("a is set twice.");
  });

  it("a property that cannot be copied is shown with its reason and only offers removal", () => {
    render(<Editor initial={[{ key: 1, name: "legacy", type: "string", value: "", locked: "Its type (other) can't be sent from here." }]} />);
    const row = screen.getByRole("group", { name: "Property 1" });
    expect(within(row).getByText("legacy")).toBeInTheDocument();
    expect(within(row).getByText(/not copyable/)).toBeInTheDocument();
    expect(within(row).queryByRole("textbox")).toBeNull();
    expect(within(row).getByRole("button", { name: "Remove legacy" })).toBeInTheDocument();
  });
});

describe("property rules", () => {
  it("follow the CLI parser", () => {
    expect(checkValue("long", "9007199254740993")).toBeNull();
    expect(checkValue("long", "9223372036854775808")).toMatch(/not a valid long/);
    expect(checkValue("double", "1e400")).toMatch(/not a valid double/);
    expect(checkValue("bool", "yes")).toMatch(/true or false/);
    expect(checkValue("datetime", "2026-10-17T12:00:00Z")).toBeNull();
    expect(checkValue("datetime", "tomorrow")).toMatch(/RFC 3339/);
    expect(checkValue("timespan", "1h30m")).toBeNull();
    expect(checkValue("timespan", "PT1H30M")).toBeNull();
    expect(checkValue("timespan", "1.02:03:04")).toBeNull();
    expect(checkValue("timespan", "soon")).toMatch(/1h30m/);
    expect(checkValue("guid", "8f0c3c55-2f2e-4b8e-9b77-1f0c2a6b3d10")).toBeNull();
    expect(checkValue("guid", "8f0c")).toMatch(/not a valid guid/);
  });

  it("increment keeps the zero padding of the trailing digits", () => {
    expect(incrementId("order-0009", 0)).toBe("order-0009");
    expect(incrementId("order-0009", 1)).toBe("order-0010");
    expect(incrementId("order-99", 1)).toBe("order-100");
    expect(incrementId("order", 0)).toBe("order");
    expect(incrementId("order", 2)).toBe("order-2");
  });
});
