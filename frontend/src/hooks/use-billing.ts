import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  type BillingWalletResponse,
  type TopUpHistoryResponse,
  type BillingUsagePeriod,
  type BillingUsageResponse,
  type ProvisionBillingWalletRequest,
  type TopUpBillingRequest,
  type TopUpBillingResponse,
  billingWalletResponseSchema,
  currentBillSchema,
  usageInvoicesSchema,
  invoiceDownloadResponseSchema,
  topUpHistoryResponseSchema,
  billingUsageResponseSchema,
  topUpBillingResponseSchema,
} from "@/schemas/billing";

export function billingOwnerPath(path: string, ownerId?: string): string {
  return ownerId ? `${path}${path.includes("?") ? "&" : "?"}owner_id=${encodeURIComponent(ownerId)}` : path;
}
function ownerKey(key: readonly unknown[], ownerId?: string) {
  return ownerId ? [...key, { ownerId }] : key;
}

function checkedOwner<T extends { owner_id: string }>(data: T, ownerId?: string): T {
  if (ownerId && data.owner_id !== ownerId) throw new Error("The server did not return the selected organization's billing data. Retry shortly.");
  return data;
}

const BILLING_WALLET_KEY = ["billing", "wallet"] as const;
const BILLING_BILL_KEY = ["billing", "bill"] as const;
const BILLING_INVOICES_KEY = ["billing", "invoices"] as const;
export const BILLING_INVOICES_PAGE_SIZE = 10;
const BILLING_USAGE_KEY = ["billing", "usage"] as const;

export function billingUsagePath(
  period?: BillingUsagePeriod,
  bucket?: "day",
  ownerId?: string,
): string {
  const params = new URLSearchParams();
  if (period) params.set("period", period);
  if (bucket) params.set("bucket", bucket);
  if (ownerId) params.set("owner_id", ownerId);
  const query = params.toString();
  return query ? `/billing/usage?${query}` : "/billing/usage";
}

export function useBillingWallet(ownerId?: string) {
  return useQuery({
    queryKey: ownerKey(BILLING_WALLET_KEY, ownerId),
    queryFn: async (): Promise<BillingWalletResponse | null> => {
      const response = await api.get<unknown>(billingOwnerPath("/billing/wallet", ownerId));
      const wallet = billingWalletResponseSchema.nullable().parse(response);
      return wallet === null ? null : checkedOwner(wallet, ownerId);
    },
    retry: false,
  });
}

export function useProvisionBillingWallet() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (
      body: ProvisionBillingWalletRequest = {},
    ): Promise<BillingWalletResponse> => {
      const response = await api.post<unknown>("/billing/wallet", body);
      return checkedOwner(billingWalletResponseSchema.parse(response), body.owner_id);
    },
    onSuccess: (wallet, body) => {
      queryClient.setQueryData(ownerKey(BILLING_WALLET_KEY, body?.owner_id), wallet);
      void queryClient.invalidateQueries({ queryKey: BILLING_USAGE_KEY });
      void queryClient.invalidateQueries({ queryKey: BILLING_BILL_KEY });
      void queryClient.invalidateQueries({ queryKey: BILLING_INVOICES_KEY });
    },
  });
}

export function useTopUpBilling() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (
      body: TopUpBillingRequest,
    ): Promise<TopUpBillingResponse> => {
      const response = await api.post<unknown>("/billing/topup", body);
      return checkedOwner(topUpBillingResponseSchema.parse(response), body.owner_id);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: BILLING_WALLET_KEY });
      void queryClient.invalidateQueries({ queryKey: BILLING_USAGE_KEY });
      void queryClient.invalidateQueries({ queryKey: BILLING_BILL_KEY });
      void queryClient.invalidateQueries({ queryKey: BILLING_INVOICES_KEY });
    },
  });
}

export function useBillingUsage(period?: BillingUsagePeriod, bucket?: "day", ownerId?: string) {
  return useQuery({
    queryKey: ownerKey(period
      ? [...BILLING_USAGE_KEY, period, ...(bucket ? [bucket] : [])]
      : BILLING_USAGE_KEY, ownerId),
    queryFn: async (): Promise<BillingUsageResponse> => {
      const response = await api.get<unknown>(billingUsagePath(period, bucket, ownerId));
      return checkedOwner(billingUsageResponseSchema.parse(response), ownerId);
    },
  });
}

export const BILLING_TOPUPS_KEY = ["billing", "topups"] as const;

export function useTopUpHistory(page = 1, perPage = 10, period = "all", ownerId?: string) {
  return useQuery({
    queryKey: ownerKey([...BILLING_TOPUPS_KEY, page, perPage, period], ownerId),
    queryFn: async (): Promise<TopUpHistoryResponse> => {
      const response = await api.get<unknown>(
        billingOwnerPath(`/billing/topups?page=${page}&per_page=${perPage}&period=${encodeURIComponent(period)}`, ownerId),
      );
      return checkedOwner(topUpHistoryResponseSchema.parse(response), ownerId);
    },
    retry: false,
    placeholderData: (previous, query) => {
      const previousOwner = query?.queryKey.at(-1);
      const sameOwner = ownerId ? JSON.stringify(previousOwner) === JSON.stringify({ ownerId }) : typeof previousOwner !== "object";
      return sameOwner ? previous : undefined;
    },
  });
}

/** Resolve the signed receipt URL for an invoice and open it in a new tab. */
export async function openInvoiceReceipt(lagoInvoiceId: string, ownerId?: string): Promise<void> {
  const response = await api.get<unknown>(
    billingOwnerPath(`/billing/invoices/${encodeURIComponent(lagoInvoiceId)}/download`, ownerId),
  );
  const { file_url } = invoiceDownloadResponseSchema.parse(response);
  window.open(file_url, "_blank", "noopener,noreferrer");
}

export function useCurrentBill(enabled = true, ownerId?: string) {
  return useQuery({
    queryKey: ownerKey(BILLING_BILL_KEY, ownerId),
    queryFn: async () =>
      currentBillSchema.parse(await api.get<unknown>(billingOwnerPath("/billing/bill", ownerId))),
    enabled,
    retry: false,
  });
}
export function useBillingInvoices(page = 1, enabled = true, ownerId?: string) {
  return useQuery({
    queryKey: ownerKey([...BILLING_INVOICES_KEY, page], ownerId),
    queryFn: async () => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(BILLING_INVOICES_PAGE_SIZE),
      });
      return usageInvoicesSchema.parse(
        await api.get<unknown>(billingOwnerPath(`/billing/invoices?${params}`, ownerId)),
      );
    },
    enabled,
    retry: false,
  });
}
