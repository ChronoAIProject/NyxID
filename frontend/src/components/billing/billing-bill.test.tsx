import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BillingBill } from "./billing-bill";
import { currentBill, usageInvoices } from "@/test/billing-bill-fixture";
const mocks = vi.hoisted(() => ({
  bill: vi.fn(),
  history: vi.fn(),
  receipt: vi.fn(),
  retry: vi.fn(),
}));
vi.mock("@/hooks/use-billing", () => ({
  BILLING_INVOICES_PAGE_SIZE: 10,
  useCurrentBill: mocks.bill,
  useBillingInvoices: mocks.history,
  openInvoiceReceipt: mocks.receipt,
}));
const query = (data: unknown) => ({
  data,
  isLoading: false,
  isError: false,
  isFetching: false,
  refetch: mocks.retry,
});
beforeEach(() => {
  vi.clearAllMocks();
  mocks.bill.mockReturnValue(query(currentBill));
  mocks.history.mockReturnValue(query(usageInvoices));
});
afterEach(() => vi.restoreAllMocks());
describe("BillingBill", () => {
  it("shows charges, GST, rounding and prepaid invoice totals; downloads an owned receipt", async () => {
    render(<BillingBill enabled />);
    expect(screen.getByText("AI requests")).toBeInTheDocument();
    expect(screen.getAllByText("GST (9%)")).toHaveLength(2);
    expect(screen.getByText("Tax rounding")).toBeInTheDocument();
    expect(screen.getByText("Prepaid credits applied")).toBeInTheDocument();
    expect(
      screen.getByText(/Usage covered by grants or allowances is excluded/),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Download receipt" }),
    );
    expect(mocks.receipt).toHaveBeenCalledWith("invoice-1");
  });
  it("uses currency amounts, trimmed rates, UTC dates and human payment labels", () => {
    const original = Date.prototype.toLocaleDateString;
    const dates = vi
      .spyOn(Date.prototype, "toLocaleDateString")
      .mockImplementation(function (this: Date, _locale, options) {
        return original.call(this, "en-US", options);
      });
    mocks.bill.mockReturnValue(
      query({
        ...currentBill,
        taxes: [{ ...currentBill.taxes[0], rate_percent: "9.000" }],
      }),
    );
    render(<BillingBill enabled />);
    expect(
      screen.getByText(
        new Intl.NumberFormat(undefined, {
          style: "currency",
          currency: "USD",
        }).format(10.91),
      ),
    ).toBeInTheDocument();
    expect(screen.getAllByText("GST (9%)")).toHaveLength(2);
    expect(screen.getByText(/10\/1\/2026 – 10\/31\/2026/)).toBeInTheDocument();
    expect(screen.getByText("9/30/2026 · Paid")).toBeInTheDocument();
    expect(dates).toHaveBeenCalledWith(undefined, { timeZone: "UTC" });
    expect(screen.queryByText(/succeeded/)).not.toBeInTheDocument();
  });
  it.each([
    ["pending", "Pending"],
    ["failed", "Failed"],
  ])("labels %s invoices as %s", (status, label) => {
    mocks.history.mockReturnValue(
      query({
        ...usageInvoices,
        invoices: [{ ...usageInvoices.invoices[0], payment_status: status }],
      }),
    );
    render(<BillingBill enabled />);
    expect(screen.getByText(new RegExp(`· ${label}$`))).toBeInTheDocument();
  });
  it("shows only aggregate tax when the breakdown is unavailable", () => {
    mocks.bill.mockReturnValue(
      query({
        ...currentBill,
        taxes: [],
        tax_breakdown_available: false,
        rounding: "0",
      }),
    );
    mocks.history.mockReturnValue(query({ ...usageInvoices, invoices: [] }));
    render(<BillingBill enabled />);
    expect(screen.getByText("Tax")).toBeInTheDocument();
    expect(screen.queryByText("GST (9%)")).not.toBeInTheDocument();
    expect(screen.queryByText("Tax rounding")).not.toBeInTheDocument();
  });
  it("has empty states for a new customer", () => {
    mocks.bill.mockReturnValue(query({ ...currentBill, charges: [] }));
    mocks.history.mockReturnValue(
      query({ ...usageInvoices, total: 0, invoices: [] }),
    );
    render(<BillingBill enabled />);
    expect(
      screen.getByText("No wallet-funded usage this billing period."),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/No finalized usage invoices yet/),
    ).toBeInTheDocument();
  });
  it("provides independent retry actions and loading states", async () => {
    mocks.bill.mockReturnValue({ ...query(undefined), isError: true });
    mocks.history.mockReturnValue({ ...query(undefined), isLoading: true });
    render(<BillingBill enabled />);
    expect(
      screen.getByText("The current bill could not be loaded."),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(mocks.retry).toHaveBeenCalledOnce();
  });
  it("does not expose bill UI before rollout is ready", () => {
    render(<BillingBill enabled={false} />);
    expect(screen.queryByText("Current bill")).not.toBeInTheDocument();
    expect(mocks.bill).toHaveBeenCalledWith(false);
    expect(mocks.history).toHaveBeenCalledWith(1, false);
  });
});
