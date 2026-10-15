import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { LabelSwatch } from "./label-swatch";
import { Segmented } from "./segmented";

describe("Segmented", () => {
  it("selects one option and never deselects", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Segmented label="Theme" value="dark" onChange={onChange} options={[{ value: "dark", label: "Dark" }, { value: "light", label: "Light" }]} />);
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(onChange).toHaveBeenCalledWith("light");
    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(onChange).not.toHaveBeenCalledWith("");
  });
});

describe("LabelSwatch", () => {
  it("renders for a known colour and nothing for none or unknown", () => {
    const { container, rerender } = render(<LabelSwatch color="blue" />);
    expect(container.firstChild).not.toBeNull();
    rerender(<LabelSwatch color={null} />);
    expect(container.firstChild).toBeNull();
    rerender(<LabelSwatch color="chartreuse" />);
    expect(container.firstChild).toBeNull();
  });
});
