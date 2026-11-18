import { z } from "zod";
import { ApiError } from "../../lib/api/ApiError";
import {
  allowancesSchema, billingOrganizationsSchema, billingPeriodSchema,
  grantsSchema, usageSchema, walletSchema, type BillingPeriod,
} from "./billingContracts";

export type BillingRead<T> = { kind: "ready"; data: T } | { kind: "disabled" } | { kind: "unsupported" };
type ReadOptions = { method: "GET"; signal?: AbortSignal };
export type BillingRequest = (path: string, options: ReadOptions) => Promise<unknown>;

/** The boundary admits GET only and never provisions a wallet or creates a checkout. */
export function createBillingApi(request: BillingRequest) {
  async function read<T>(path: string, schema: z.ZodType<T>, signal?: AbortSignal): Promise<BillingRead<T>> {
    try {
      const payload = await request(path, { method: "GET", signal });
      if (payload && typeof payload === "object" && (
        ("enabled" in payload && payload.enabled === false) ||
        ("billing_enabled" in payload && payload.billing_enabled === false)
      )) return { kind: "disabled" };
      return { kind: "ready", data: schema.parse(payload) };
    } catch (cause) {
      if (cause instanceof ApiError) {
        if (cause.statusCode === 404) return { kind: "unsupported" };
        if (cause.errorKey === "billing_not_configured" || cause.errorKey === "billing_disabled" ||
          (cause.statusCode === 403 && cause.message === "Billing is not enabled for this account")) return { kind: "disabled" };
      }
      throw cause;
    }
  }
  function benefitPath(path: "grants" | "allowances", ownerId?: string) {
    return `/billing/${path}${ownerId ? `?owner_id=${encodeURIComponent(z.string().uuid().parse(ownerId))}` : ""}`;
  }
  return {
    wallet(signal?: AbortSignal) {
      // The legacy GET /wallet provisions absent wallets. Older servers return
      // 404 for this distinct read route; never fall back to the legacy GET.
      return read("/billing/wallet/view", walletSchema.nullable(), signal);
    },
    async usage(period: BillingPeriod, signal?: AbortSignal) {
      const result = await read(`/billing/usage?period=${billingPeriodSchema.parse(period)}`, usageSchema, signal);
      if (result.kind === "ready" && result.data.period !== period) throw new Error("The server returned a different usage period");
      return result;
    },
    grants(ownerId?: string, signal?: AbortSignal) {
      return read(benefitPath("grants", ownerId), grantsSchema, signal);
    },
    allowances(ownerId?: string, signal?: AbortSignal) {
      return read(benefitPath("allowances", ownerId), allowancesSchema, signal);
    },
    organizations(signal?: AbortSignal) {
      return read("/orgs", billingOrganizationsSchema, signal);
    },
  };
}
