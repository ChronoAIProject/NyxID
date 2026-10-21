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
}: {
  readonly quota?: z.infer<typeof remoteMachineQuotaSchema> | null;
}) {
  const now = useObservationTime();
  const stale = !quota?.valid_until || Date.parse(quota.valid_until) <= now;
  return (
    <section
      aria-label="Platform quota preflight"
      className="space-y-1 text-12 text-muted-foreground"
    >
      <p>
        Namespace quota: {quota ? quota.status.replaceAll("_", " ") : "Unknown"}
        {stale ? " (no fresh observation)" : ""}.
      </p>
      {quota && (
        <>
          <p>
            {quota.operation === "retained_volume_resume"
              ? "Resume demand reuses the retained workspace volume."
              : "Create demand includes a new workspace volume."}{" "}
            {quota.reason.replaceAll("_", " ")}.
          </p>
          {quota.additional_profiles_upper_bound !== null && (
            <p>
              Observed quota upper bound:{" "}
              {quota.additional_profiles_upper_bound} additional fixed profiles.
            </p>
          )}
        </>
      )}
      <p>
        Quota headroom is not a capacity guarantee. Controller-reported usage
        may lag; scheduling and storage availability are checked during
        provisioning. Temporary disk capacity and per-Machine quota demand
        remain unknown; the absence of a fixed declaration does not establish
        available node disk space.
      </p>
    </section>
  );
}

export function RemoteMachineBillingPreflight({
  preflight,
  quoteProfile,
  quoteRate,
  quotePrice,
  now,
}: {
  readonly preflight?: z.infer<
    typeof remoteMachineBillingPreflightSchema
  > | null;
  readonly quoteProfile?: string | null;
  readonly quoteRate?: string | null;
  readonly quotePrice?: string | null;
  readonly now?: number;
}) {
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
      aria-label="Paid runtime prerequisites"
      className="space-y-2 text-12 text-muted-foreground"
    >
      <p>
        Billing preflight:{" "}
        {!preflight || !matches
          ? "Unknown for this quote"
          : !fresh
            ? "Stale; submit checks current state"
            : preflight.ready
              ? "Prerequisites observed ready"
              : "Not ready"}
        .
      </p>
      {preflight && matches && (
        <>
          <p>
            Wallet: {preflight.wallet}. Subscription:{" "}
            {preflight.subscription.replaceAll("_", " ")}. Provider:{" "}
            {preflight.provider}. Exact accounting:{" "}
            {preflight.exact_ready ? "ready" : "not ready"}. Pricing:{" "}
            {preflight.pricing}.
          </p>
          {preflight.reason && <p>{preflight.reason.replaceAll("_", " ")}.</p>}
        </>
      )}
      {funds ? (
        <>
          <p>
            Quote-specific wallet available: {funds.wallet_available} Credits.
            Reserved: {funds.wallet_reserved} Credits. Pending debits:{" "}
            {funds.pending_debits} Credits. Pending expiry:{" "}
            {funds.pending_expiry} Credits.
          </p>
          <p>
            Applicable benefits: {funds.matching_allowance_seconds} running
            seconds and {funds.available_grants} grant Credits.
          </p>
          <p>
            Estimated runtime at this rate: {funds.estimated_seconds} seconds.
          </p>
        </>
      ) : (
        <p>
          Quote-specific benefits and runtime estimate: unavailable.{" "}
          {preflight?.funds_reason?.replaceAll("_", " ")}
        </p>
      )}
      <p>
        Other Machines and services share these funds. The estimate reserves
        nothing and does not guarantee a duration; submission repeats live
        eligibility, subscription, provider and funding checks.
      </p>
    </section>
  );
}
