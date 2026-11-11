import { translateUi } from "@/i18n/use-ui-translation";
import { metricLabel } from "@/schemas/billing-metrics";
import type {
  ConfiguredCatalogEntry,
  ServiceBillingExplanation,
} from "@/schemas/service-insights";
import { configuredPlatformPrice } from "./service-billing-config";
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
  catalog?: ConfiguredCatalogEntry,
): PlainBilling {
  const provider = providerName(connection);
  const org =
    connection.credential_source?.type === "org"
      ? connection.credential_source.org_name
      : null;
  const supplier = supplierOf(bill);
  const oauth = OAUTH.includes(connection.credential_type);
  const master = bill.credential_class === "nyxid_managed_master";
  const thing = oauth ? translateUi("app") : translateUi("key");
  const owner = org ? `${org}'s` : translateUi("Your");
  const you = org ?? translateUi("you");

  const key =
    supplier === "nyxid"
      ? master
        ? { title: translateUi("NyxID's {{value0}} key", { value0: provider }), note: translateUi("NyxID provides the key.") }
        : {
            title: translateUi("NyxID's {{value0}} app", { value0: provider }),
            note: translateUi("You sign in with your own {{value0}} account; NyxID provides the app.", { value0: provider }),
          }
      : supplier === "own"
        ? {
            title:
              connection.credential_type === "node_managed" ||
              bill.credential_class === "node_managed"
                ? translateUi("A key stored on your node")
                : oauth
                  ? `${owner} own ${provider} app`
                  : `${owner} own API key`,
          }
        : supplier === "none"
          ? { title: translateUi("No key needed") }
          : {
              title: translateUi("Not confirmed"),
              note: oauth
                ? translateUi("We can't tell yet whether this sign-in uses your own {{value0}} app or NyxID's.", { value0: provider })
                : translateUi("We can't tell who supplied this key."),
            };

  if (bill.status === "restricted" || bill.status === "unavailable") {
    const restricted = bill.status === "restricted";
    return {
      verdict: "hidden",
      headline: restricted
        ? translateUi("You can't see billing for this connection")
        : translateUi("Billing couldn't load"),
      detail: restricted
        ? translateUi("Ask the owner of this connection what it costs.")
        : translateUi("Try again later."),
      key,
      payer: translateUi("Not available"),
      price: translateUi("Not available"),
      short: translateUi("Billing unavailable"),
      tips: [],
    };
  }

  // A connection using NyxID's key or app is billed at the service's NyxID
  // rate, shown the same way for every NyxID-supplied service.
  const servicePrice =
    supplier === "nyxid" && !bill.rates.length
      ? configuredPlatformPrice(connection, catalog)
      : null;
  const free =
    !servicePrice &&
    (bill.credit_billing_configured === false ||
      bill.charge_status === "not_charged");
  const rates = servicePrice
    ? [
        {
          layer: "platform",
          metric: servicePrice.metric,
          credits_per_unit: servicePrice.credits_per_unit,
          currency: "credits",
          source: "configuration",
          sync_status: servicePrice.sync_status,
        },
      ]
    : bill.rates;
  const unit = (metric: string) => metricLabel(metric, 1);
  const price = free
    ? translateUi("Free on NyxID")
    : rates.length
      ? rates
          .map((rate) =>
            rate.credits_per_unit == null
              ? translateUi("Plan price per {{value0}}", { value0: unit(rate.metric) })
              : `${rate.credits_per_unit} credits per ${unit(rate.metric)}`,
          )
          .join(" + ")
      : translateUi("Not confirmed");
  const shortPrice =
    rates.length === 1 && rates[0]!.credits_per_unit != null
      ? `${rates[0]!.credits_per_unit} credits/${unit(rates[0]!.metric)}`
      : translateUi("NyxID credits");

  if (free) {
    const detail =
      supplier === "nyxid"
        ? translateUi("This connection uses NyxID's {{value0}} {{value1}}, and NyxID doesn't charge for it right now.", { value0: provider, value1: thing })
        : supplier === "own"
          ? translateUi("This connection uses {{value0}} own {{value1}}, so NyxID doesn't charge for it. {{value2}} may bill {{value3}} directly.", { value0: org ? owner : "your", value1: thing, value2: provider, value3: you })
          : supplier === "none"
            ? translateUi("This service doesn't need a key, and NyxID doesn't charge for it.")
            : translateUi("NyxID doesn't charge for this connection.");
    return {
      verdict: "free",
      headline: translateUi("Free on NyxID"),
      detail,
      key,
      payer: translateUi("No one. NyxID doesn't charge for this."),
      price,
      short: translateUi("Free on NyxID"),
      tips: [],
    };
  }

  if (supplier === "unknown") {
    return {
      verdict: "unconfirmed",
      headline: translateUi("Cost not confirmed"),
      detail: translateUi("NyxID charges for {{value0}} when you use NyxID's {{value1}}, but not when you use your own. We can't tell yet which one this connection uses.", { value0: provider, value1: thing }),
      key,
      payer: translateUi("Depends on whose app is used"),
      price,
      short: translateUi("Cost not confirmed"),
      tips: [],
    };
  }

  const payer = bill.account
    ? bill.account.kind === "personal"
      ? translateUi("You, from your personal credits")
      : `${bill.account.name}, from its organization credits`
    : master
      ? translateUi("Whoever makes the call, from their personal credits")
      : org
        ? `${org}, from its organization credits`
        : translateUi("You, from your personal credits");
  const payerShort = bill.account
    ? bill.account.kind === "personal"
      ? translateUi("You pay")
      : `${bill.account.name} pays`
    : master
      ? translateUi("Caller pays")
      : org
        ? `${org} pays`
        : translateUi("You pay");
  const each =
    rates.length === 1 && rates[0]!.credits_per_unit != null
      ? translateUi("Each {{value0}} costs {{value1}} NyxID credits", { value0: unit(rates[0]!.metric), value1: rates[0]!.credits_per_unit })
      : translateUi("Usage costs NyxID credits");
  const payerPhrase =
    payer.startsWith("You") || payer.startsWith("Whoever")
      ? payer.charAt(0).toLowerCase() + payer.slice(1)
      : payer;
  const tips = [
    translateUi("Credits are used in this order: free allowances first, then credit grants, then your wallet balance."),
    ...(rates.some((rate) => rate.sync_status && rate.sync_status !== "synced")
      ? [
          translateUi("A new price is waiting to be activated. Until then, the previous price applies."),
        ]
      : []),
    ...(rates.some((rate) => rate.credits_per_unit == null)
      ? [translateUi("The price comes from your billing plan and isn't shown here.")]
      : []),
    ...(bill.context === "configuration"
      ? [translateUi("Agent keys set to use a different key can be billed differently.")]
      : []),
  ];
  return {
    verdict: "charged",
    headline: translateUi("Uses NyxID credits"),
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
