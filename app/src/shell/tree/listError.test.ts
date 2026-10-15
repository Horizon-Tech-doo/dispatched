import { describe, expect, it } from "vitest";
import { describeListError } from "./listError";

describe("describeListError", () => {
  it("explains a network failure and the next step", () => {
    expect(describeListError("network", "sb-logistics-prod")).toBe("The connection to sb-logistics-prod was interrupted. Check your network or VPN, then retry.");
    expect(describeListError("timeout", "h")).toContain("interrupted");
  });
  it("explains missing rights", () => {
    expect(describeListError("forbidden", "sb-logistics-prod")).toBe(
      "This identity isn't allowed to list sb-logistics-prod. Grant it Manage rights or pin entities on the connection.",
    );
  });
  it("falls back to a general sentence", () => {
    expect(describeListError("unknown", "h")).toBe("Something went wrong listing h. Retry, or check the activity log for details.");
  });
});
