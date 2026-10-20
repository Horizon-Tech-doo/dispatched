import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { RateChart } from "./RateChart";

describe("rate chart", () => {
  it("rate_chart_has_summary_label", () => {
    const messages = Array.from({ length: 60 }, (_, i) => (i === 59 ? 12 : i === 30 ? 40 : 0));
    const bytes = messages.map((n) => n * 1024);
    render(<RateChart messages={messages} bytes={bytes} />);
    expect(screen.getByRole("img", { name: /Messages per second: 12 now, peak 40 in the last minute/ })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: /Bytes per second: 12(\.0)? KiB now, peak 40(\.0)? KiB/ })).toBeInTheDocument();
  });
});
