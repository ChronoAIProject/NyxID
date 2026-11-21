import { translateApp } from "../../i18n/appTranslation";
import type { BillingAllowance, BillingGrant, BillingWallet } from "./billingContracts";
import { billingDate, creditUnits, decimalCredits, formatExactCredits, formatQuantity } from "./credits";
import { billingMetricName } from "./billingMetric";

export type BalanceRow = {
  key: string; label: string; value: string; exactValue?: string; unit?: string;
  /** Allowances list under their service with a short metric name. */
  group?: string; item?: string; left?: string;
};
const positive = (value: string | null | undefined) => value != null && creditUnits(value) > 0n;
const maxZero = (value: bigint) => (value > 0n ? value : 0n);
const min = (a: bigint, b: bigint) => (a < b ? a : b);

/** Mirrors the grant's held balance and disjoint service ceilings, without
 * presenting service-restricted credits as unrestricted wallet credits. */
export function spendableGrant(grant: BillingGrant, now: number): bigint {
  if (grant.status !== "active" || grant.activation_state !== "active" || grant.revoke_requested_at ||
    (grant.expires_at && Date.parse(grant.expires_at) <= now) || grant.remaining === null) return 0n;
  const held = grant.held ?? grant.reserved;
  if (held === null) return 0n;
  const available = maxZero(creditUnits(grant.remaining) - maxZero(creditUnits(held)));
  if (grant.scope.all_services) return available;
  const refs = grant.scope.service_ids.length ? grant.scope.service_ids : grant.scope.service_slugs;
  const field = grant.scope.service_ids.length ? "service_ids" : "service_slugs";
  if (!refs.length) return 0n;
  const covered = refs.every((ref) => grant.service_limits.some((rule) => rule[field].includes(ref)));
  if (!covered) return available;
  const cap = grant.service_limits.filter((rule) => rule[field].some((ref) => refs.includes(ref)))
    .reduce((sum, rule) => sum + maxZero(creditUnits(rule.limit) - creditUnits(rule.consumed) - creditUnits(rule.held ?? rule.reserved)), 0n);
  return min(available, cap);
}

export function balanceRows(wallet: BillingWallet, grants: BillingGrant[], allowances: BillingAllowance[], now: number): BalanceRow[] {
  const rows: BalanceRow[] = [];
  function credits(key: string, label: string, value: string | null) {
    if (positive(value)) rows.push({ key, label, value: formatExactCredits(value!), exactValue: value! });
  }
  credits("limit", translateApp("Credit limit"), wallet.overdraftCap);
  const pending = [wallet.reserved, wallet.pendingDebits].filter((value): value is string => value !== null);
  credits("pending",
    translateApp("Pending / reserved"), decimalCredits(pending.reduce((sum, value) => sum + maxZero(creditUnits(value)), 0n)));
  credits("expiry-pending",
    translateApp("Pending expiry"), wallet.pendingExpiry);

  const included = new Map<string, bigint>();
  let expiry: number | undefined;
  let expiring = 0n;
  for (const grant of grants) {
    const amount = spendableGrant(grant, now);
    if (amount <= 0n) continue;
    const services = grant.scope.service_slugs.join(", ") || translateApp("Selected services");
    const label = translateApp(grant.scope.all_services
      ? grant.service_limits.length ? "Included credits · service limits apply" : "Included credits"
      : grant.service_limits.length ? "Included credits · {{services}} · service limits apply" : "Included credits · {{services}}",
      { services });
    included.set(label, (included.get(label) ?? 0n) + amount);
    const date = grant.expires_at ? Date.parse(grant.expires_at) : undefined;
    if (date !== undefined && date <= now + 30 * 86_400_000) {
      if (expiry === undefined || date < expiry) { expiry = date; expiring = amount; }
      else if (date === expiry) expiring += amount;
    }
  }
  for (const [label, amount] of included) credits(label, label, decimalCredits(amount));
  if (expiry !== undefined) credits("expiring",
      translateApp("Expiring {{date}} (UTC)", {
        date: billingDate(new Date(expiry).toISOString()),
      }), decimalCredits(expiring));
  for (const balance of allowances) {
    const { allowance } = balance;
    if (!allowance.is_active || balance.remaining_quantity === 0 || Date.parse(balance.period_start) > now ||
      (balance.period_end && Date.parse(balance.period_end) <= now)) continue;
    const period = { one_time: "", daily: translateApp("Today"), weekly: translateApp("This week"), monthly: translateApp("This month") }[allowance.recurrence];
    const remainingTemplate = {
      one_time: "{{remaining}} / {{quantity}} {{metric}} left",
      daily: "{{remaining}} / {{quantity}} {{metric}} left today",
      weekly: "{{remaining}} / {{quantity}} {{metric}} left this week",
      monthly: "{{remaining}} / {{quantity}} {{metric}} left this month",
    }[allowance.recurrence];
    const metric = billingMetricName(allowance.metric);
    rows.push({ key: `allowance:${allowance.id}`, label: translateApp("Included allowance · {{service}}", {
        service: allowance.service_slug,
      }),
      value: translateApp(remainingTemplate, {
        remaining: formatQuantity(balance.remaining_quantity), quantity: formatQuantity(allowance.quantity), metric,
      }), unit: "",
      group: period ? translateApp("{{service}} · {{period}}", { service: allowance.service_slug, period }) : allowance.service_slug, item: metric.charAt(0).toUpperCase() + metric.slice(1),
      left: translateApp("{{remaining}} of {{quantity}} left", {
        remaining: formatQuantity(balance.remaining_quantity),
        quantity: formatQuantity(allowance.quantity),
      }),
    });
  }
  return rows;
}
