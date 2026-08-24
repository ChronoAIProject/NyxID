import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  billingUsagePath,
  useBillingUsage,
  useBillingWallet,
  useProvisionBillingWallet,
  useTopUpBilling,
} from "./use-billing";

const { mockGet, mockPost } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, post: mockPost },
}));

function wrapperFactory() {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe("billingUsagePath", () => {
  it("targets only the read-only billing usage endpoint", () => {
    expect(billingUsagePath()).toBe("/billing/usage");
    expect(billingUsagePath("7d")).toBe("/billing/usage?period=7d");
  });
});

describe("useBillingUsage", () => {
  it("fetches and validates read-only billing usage", async () => {
    mockGet.mockResolvedValue({
      owner_id: "owner-1",
      period: "7d",
      rows: [],
      totals: {
        quantity: 0,
        requests: 0,
        bytes: 0,
        events: 0,
        estimated_credits_micros: null,
      },
      billing: {
        charging_enabled: false,
        lago_configured: false,
        source: "usage_meter",
        rates_are_approximate: true,
      },
    });

    const { result } = renderHook(() => useBillingUsage("7d"), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/billing/usage?period=7d");
    expect(result.current.data?.billing.charging_enabled).toBe(false);
  });

  it("rejects wallet or top-up-shaped payloads", async () => {
    mockGet.mockResolvedValue({
      owner_id: "owner-1",
      period: "7d",
      rows: [],
      totals: {
        quantity: 0,
        requests: 0,
        bytes: 0,
        events: 0,
      },
      billing: {
        charging_enabled: true,
        lago_configured: true,
        source: "lago_wallet",
        rates_are_approximate: false,
      },
    });

    const { result } = renderHook(() => useBillingUsage("7d"), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isError).toBe(true));
  });
});

describe("billing wallet hooks", () => {
  const wallet = {
    owner_id: "owner-1",
    plan_kind: "prepaid",
    collection_state: "good",
    balance_credits: 100,
    reserved_credits: 10,
    pending_lago_debits: 5,
    pending_topup_expiry_credits: 0,
    available_credits: 85,
    available_with_overdraft_credits: 85,
    has_payment_instrument: false,
    overdraft_cap_credits: 0,
    suspended: false,
    lago_customer_id: "customer-1",
    lago_subscription_id: "subscription-1",
    lago_wallet_id: "wallet-1",
    balance_synced_at: "2026-06-26T00:00:00Z",
    created_at: "2026-06-26T00:00:00Z",
    updated_at: "2026-06-26T00:00:00Z",
    created: false,
  };

  it("fetches and validates the billing wallet", async () => {
    mockGet.mockResolvedValue(wallet);

    const { result } = renderHook(() => useBillingWallet(), {
      wrapper: wrapperFactory(),
    });

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockGet).toHaveBeenCalledWith("/billing/wallet");
    expect(result.current.data?.available_credits).toBe(85);
  });

  it("provisions the billing wallet with the server-owned endpoint", async () => {
    mockPost.mockResolvedValue({ ...wallet, created: true });

    const { result } = renderHook(() => useProvisionBillingWallet(), {
      wrapper: wrapperFactory(),
    });

    result.current.mutate({});
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockPost).toHaveBeenCalledWith("/billing/wallet", {});
    expect(result.current.data?.created).toBe(true);
  });

  it("creates a top-up checkout", async () => {
    mockPost.mockResolvedValue({
      owner_id: "owner-1",
      amount_credits: 50,
      idempotency_key: "topup-12345678",
      checkout_url: "https://checkout.example.com/session",
      payment_provider: "stripe",
      lago_wallet_transaction_id: "txn-1",
      lago_invoice_id: "invoice-1",
      status: "checkout_created",
      reused: false,
    });

    const { result } = renderHook(() => useTopUpBilling(), {
      wrapper: wrapperFactory(),
    });

    result.current.mutate({
      amount_credits: 50,
      idempotency_key: "topup-12345678",
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(mockPost).toHaveBeenCalledWith("/billing/topup", {
      amount_credits: 50,
      idempotency_key: "topup-12345678",
    });
    expect(result.current.data?.checkout_url).toBe("https://checkout.example.com/session");
  });
});
