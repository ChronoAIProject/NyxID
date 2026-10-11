import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { AdminBillingPage } from "./admin-billing";

vi.mock("@/hooks/use-admin-billing", () => ({
  useAdminBillingOverview: () => ({ data: [{ id: "1", slug: "svc", name: "Service", has_billing: true, effective_source: "service" }] }),
  useAdminBillingRates: () => ({ data: [] }),
  useAdminBillingConfig: () => ({ data: { billing_enabled: true, lago_configured: false, resale_enabled: false, global: null, precedence: ["env", "global", "service", "legacy/free"] } }),
  useResyncAdminBillingService: () => ({ mutate: vi.fn() }),
  useUpdateAdminBillingConfig: () => ({ mutate: vi.fn() }),
}));

describe("AdminBillingPage", () => {
  it("renders overview and all three tabs", () => {
    render(<AdminBillingPage />);
    expect(screen.getByText("Service")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Overview" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Rates" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
  });
});
