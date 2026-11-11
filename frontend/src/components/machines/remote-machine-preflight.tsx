import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useState } from "react";
import type { z } from "zod";
import { parseCredits } from "@/lib/credits";
import type {
  remoteMachineBillingPreflightSchema,
  remoteMachineQuotaSchema,
} from "@/schemas/machines";

function useObservationTime() {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, []);
  return now;
}

export function RemoteMachineQuota({
  quota,
  grouped = false,
}: {
  readonly grouped?: boolean;
  readonly quota?: z.infer<typeof remoteMachineQuotaSchema> | null;
}) {
  useUiTranslation();
  const now = useObservationTime();
  const stale = !quota?.valid_until || Date.parse(quota.valid_until) <= now;
  return (
    <section
      aria-label={translateUi("Platform quota preflight")}
      className={grouped ? "space-y-1" : "space-y-1 text-14 text-muted-foreground"}
    >
      <p>{translateUi("Namespace quota: ")}{quota ? quota.status.replaceAll("_", " ") : translateUi("Unknown")}
        {stale ? translateUi(" (no fresh observation)") : ""}.
      </p>
      {quota && (
        <>
          <p>
            {quota.operation === "retained_volume_resume"
              ? translateUi("Resume demand reuses the retained workspace volume.")
              : translateUi("Create demand includes a new workspace volume.")}{" "}
            {quota.reason.replaceAll("_", " ")}.
          </p>
          {quota.additional_profiles_upper_bound !== null && (
            <p>{translateUi("Observed quota upper bound:")}{" "}
              {quota.additional_profiles_upper_bound}{translateUi(" additional fixed profiles.")}</p>
          )}
        </>
      )}
      <p>{translateUi("Quota headroom is not a capacity guarantee. Controller-reported usage may lag; scheduling and storage availability are checked during provisioning. Temporary disk capacity and per-Machine quota demand remain unknown; the absence of a fixed declaration does not establish available node disk space.")}</p>
    </section>
  );
}

export function RemoteMachineBillingPreflight({
  preflight,
  grouped = false,
  quoteProfile,
  quoteRate,
  quotePrice,
  now,
}: {
  readonly grouped?: boolean;
  readonly preflight?: z.infer<
    typeof remoteMachineBillingPreflightSchema
  > | null;
  readonly quoteProfile?: string | null;
  readonly quoteRate?: string | null;
  readonly quotePrice?: string | null;
  readonly now?: number;
}) {
  useUiTranslation();
  const observedNow = useObservationTime();
  const matches =
    (!quoteProfile || preflight?.profile_version === quoteProfile) &&
    (!quoteRate || preflight?.rate_version === quoteRate) &&
    (!quotePrice ||
      (preflight?.credits_per_second != null &&
        parseCredits(preflight.credits_per_second) === parseCredits(quotePrice)));
  const fresh = Boolean(
    preflight && Date.parse(preflight.valid_until) > (now ?? observedNow),
  );
  const funds = matches && fresh ? preflight?.funds : undefined;
  return (
    <section
      aria-label={translateUi("Paid runtime prerequisites")}
      className={grouped ? "space-y-1" : "space-y-2 text-14 text-muted-foreground"}
    >
      <p>{translateUi("Billing preflight:")}{" "}
        {!preflight || !matches
          ? translateUi("Unknown for this quote")
          : !fresh
            ? translateUi("Stale; submit checks current state")
            : preflight.ready
              ? translateUi("Prerequisites observed ready")
              : translateUi("Not ready")}
        .
      </p>
      {preflight && matches && (
        <>
          <p>{translateUi("Wallet: ")}{preflight.wallet}. Subscription:{" "}
            {preflight.subscription.replaceAll("_", " ")}. Provider:{" "}
            {preflight.provider}. Exact accounting:{" "}
            {preflight.exact_ready ? translateUi("ready") : translateUi("not ready")}. Pricing:{" "}
            {preflight.pricing}.
          </p>
          {preflight.reason && <p>{preflight.reason.replaceAll("_", " ")}.</p>}
        </>
      )}
      {funds ? (
        <>
          <p>{translateUi("Quote-specific wallet available: ")}{funds.wallet_available}{translateUi(" Credits. Reserved: ")}{funds.wallet_reserved}{translateUi(" Credits. Pending debits:")}{" "}
            {funds.pending_debits}{translateUi(" Credits. Pending expiry:")}{" "}
            {funds.pending_expiry}{translateUi(" Credits.")}</p>
          <p>{translateUi("Applicable benefits: ")}{funds.matching_allowance_seconds}{translateUi(" running seconds and ")}{funds.available_grants}{translateUi(" grant Credits.")}</p>
          <p>{translateUi("Estimated runtime at this rate: ")}{funds.estimated_seconds}{translateUi(" seconds.")}</p>
        </>
      ) : (
        <p>{translateUi("Quote-specific benefits and runtime estimate: unavailable.")}{" "}
          {preflight?.funds_reason?.replaceAll("_", " ")}
        </p>
      )}
      <p>{translateUi("Other Machines and services share these funds. The estimate reserves nothing and does not guarantee a duration; submission repeats live eligibility, subscription, provider and funding checks.")}</p>
    </section>
  );
}
