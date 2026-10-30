export const currentBill = {
  owner_id: "owner",
  currency: "USD",
  from: "2026-10-01T00:00:00Z",
  to: "2026-10-31T23:59:59Z",
  charges: [
    {
      metric_code: "requests",
      name: "AI requests",
      units: "1.25",
      amount: "10",
    },
  ],
  subtotal: "10",
  taxes: [{ code: "sg-gst", name: "GST", rate_percent: "9", amount: "0.9" }],
  tax_breakdown_available: true,
  tax_total: "0.91",
  rounding: "0.01",
  total: "10.91",
};
export const usageInvoices = {
  owner_id: "owner",
  page: 1,
  per_page: 10,
  total: 1,
  invoices: [
    {
      id: "invoice-1",
      number: "INV-001",
      issuing_date: "2026-09-30",
      currency: "USD",
      payment_status: "succeeded",
      subtotal: "10",
      taxes: currentBill.taxes,
      tax_total: "0.9",
      total: "10.9",
      prepaid_credits_applied: "10.9",
      amount_due: "0",
    },
  ],
};
