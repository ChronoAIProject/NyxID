import { metricLabel } from "@/schemas/billing-metrics";
import type { ServiceBillingExplanation } from "@/schemas/service-insights";
import type { KeyInfo } from "@/types/keys";

export type BillingVerdict = "charged" | "free" | "unconfirmed" | "hidden";

/** Billing in words a first-time user can act on: cost, whose key, who pays. */
export interface PlainBilling {
  readonly verdict: BillingVerdict;
  readonly headline: string;
  readonly detail: string;
  readonly key: { readonly title: string; readonly note?: string };
  readonly payer: string;
  readonly price: string;
  /** Short form for table rows, e.g. "You pay · 0.05 credits/request". */
  readonly short: string;
  readonly tips: readonly string[];
}

const OAUTH = ["oauth2", "device_code"];

function providerName(connection: KeyInfo): string {
  const name =
    connection.catalog_service_name ?? connection.name ?? connection.label;
  return name.replace(/\s+API$/i, "");
}

function supplierOf(
  bill: ServiceBillingExplanation,
): "nyxid" | "own" | "none" | "unknown" {
  if (bill.credential_supplier) return bill.credential_supplier;
  switch (bill.credential_class) {
    case "nyxid_managed_master":
    case "nyxid_platform_oauth_app":
      return "nyxid";
    case "user_owned":
    case "agent_override_user_owned":
    case "node_managed":
      return "own";
    case "no_auth":
      return "none";
    default:
      return "unknown";
  }
}

export function plainBilling(
  connection: KeyInfo,
  bill: ServiceBillingExplanation,
): PlainBilling {
  const provider = providerName(connection);
  const org =
    connection.credential_source?.type === "org"
      ? connection.credential_source.org_name
      : null;
  const supplier = supplierOf(bill);
  const oauth = OAUTH.includes(connection.credential_type);
  const master = bill.credential_class === "nyxid_managed_master";
  const thing = oauth ? "app" : "key";
  const owner = org ? `${org}'s` : "Your";
  const you = org ?? "you";

  const key =
    supplier === "nyxid"
      ? master
        ? { title: `NyxID's ${provider} key`, note: "NyxID provides the key." }
        : {
            title: `NyxID's ${provider} app`,
            note: `You sign in with your own ${provider} account; NyxID provides the app.`,
          }
      : supplier === "own"
        ? {
            title:
              connection.credential_type === "node_managed" ||
              bill.credential_class === "node_managed"
                ? "A key stored on your node"
                : oauth
                  ? `${owner} own ${provider} app`
                  : `${owner} own API key`,
          }
        : supplier === "none"
          ? { title: "No key needed" }
          : {
              title: "Not confirmed",
              note: oauth
                ? `We can't tell yet whether this sign-in uses your own ${provider} app or NyxID's.`
                : "We can't tell who supplied this key.",
            };

  if (bill.status === "restricted" || bill.status === "unavailable") {
    const restricted = bill.status === "restricted";
    return {
      verdict: "hidden",
      headline: restricted
        ? "You can't see billing for this connection"
        : "Billing couldn't load",
      detail: restricted
        ? "Ask the owner of this connection what it costs."
        : "Try again later.",
      key,
      payer: "Not available",
      price: "Not available",
      short: "Billing unavailable",
      tips: [],
    };
  }

  const free =
    bill.credit_billing_configured === false ||
    bill.charge_status === "not_charged";
  const rates = bill.rates;
  const unit = (metric: string) => metricLabel(metric, 1);
  const price = free
    ? "Free on NyxID"
    : rates.length
      ? rates
          .map((rate) =>
            rate.credits_per_unit == null
              ? `Plan price per ${unit(rate.metric)}`
              : `${rate.credits_per_unit} credits per ${unit(rate.metric)}`,
          )
          .join(" + ")
      : "Not confirmed";
  const shortPrice =
    rates.length === 1 && rates[0]!.credits_per_unit != null
      ? `${rates[0]!.credits_per_unit} credits/${unit(rates[0]!.metric)}`
      : "NyxID credits";

  if (free) {
    const detail =
      supplier === "nyxid"
        ? `This connection uses NyxID's ${provider} ${thing}, and NyxID doesn't charge for it right now.`
        : supplier === "own"
          ? `This connection uses ${org ? owner : "your"} own ${thing}, so NyxID doesn't charge for it. ${provider} may bill ${you} directly.`
          : supplier === "none"
            ? "This service doesn't need a key, and NyxID doesn't charge for it."
            : "NyxID doesn't charge for this connection.";
    return {
      verdict: "free",
      headline: "Free on NyxID",
      detail,
      key,
      payer: "No one. NyxID doesn't charge for this.",
      price,
      short: "Free on NyxID",
      tips: [],
    };
  }

  if (supplier === "unknown") {
    return {
      verdict: "unconfirmed",
      headline: "Cost not confirmed",
      detail: `NyxID charges for ${provider} when you use NyxID's ${thing}, but not when you use your own. We can't tell yet which one this connection uses.`,
      key,
      payer: "Depends on whose app is used",
      price,
      short: "Cost not confirmed",
      tips: [],
    };
  }

  const payer = bill.account
    ? bill.account.kind === "personal"
      ? "You, from your personal credits"
      : `${bill.account.name}, from its organization credits`
    : master
      ? "Whoever makes the call, from their personal credits"
      : org
        ? `${org}, from its organization credits`
        : "You, from your personal credits";
  const payerShort = bill.account
    ? bill.account.kind === "personal"
      ? "You pay"
      : `${bill.account.name} pays`
    : master
      ? "Caller pays"
      : org
        ? `${org} pays`
        : "You pay";
  const each =
    rates.length === 1 && rates[0]!.credits_per_unit != null
      ? `Each ${unit(rates[0]!.metric)} costs ${rates[0]!.credits_per_unit} NyxID credits`
      : "Usage costs NyxID credits";
  const payerPhrase =
    payer.startsWith("You") || payer.startsWith("Whoever")
      ? payer.charAt(0).toLowerCase() + payer.slice(1)
      : payer;
  const tips = [
    "Credits are used in this order: free allowances first, then credit grants, then your wallet balance.",
    ...(rates.some((rate) => rate.sync_status && rate.sync_status !== "synced")
      ? [
          "A new price is waiting to be activated. Until then, the previous price applies.",
        ]
      : []),
    ...(rates.some((rate) => rate.credits_per_unit == null)
      ? ["The price comes from your billing plan and isn't shown here."]
      : []),
    ...(bill.context === "configuration"
      ? ["Agent keys set to use a different key can be billed differently."]
      : []),
  ];
  return {
    verdict: "charged",
    headline: "Uses NyxID credits",
    detail: `${each}, paid by ${payerPhrase}.${
      supplier === "own"
        ? ` This is a NyxID fee on top of anything ${provider} bills ${you} directly.`
        : ""
    }`,
    key,
    payer,
    price,
    short: `${payerShort} · ${shortPrice}`,
    tips,
  };
}
