import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import {
  RemoteMachineBillingPreflight,
  RemoteMachineQuota,
} from "./remote-machine-preflight";
import { remoteMachineBillingPreflightSchema } from "@/schemas/machines";

const preflight = remoteMachineBillingPreflightSchema.parse({
  profile_version: "remote-machine",
  ready: true,
  billing_enabled: true,
  exact_ready: true,
  wallet: "ready",
  subscription: "active",
  provider: "ready",
  pricing: "synced",
  reason: null,
  rate_version: "accepted-rate",
  credits_per_second: "0.000040",
  observed_at: "2099-01-01T00:00:00Z",
  valid_until: "2099-01-01T00:00:30Z",
  reservation_guarantee: false,
  funds_reason: null,
  funds: {
    wallet_available: "2.123456789012",
    wallet_reserved: "1",
    pending_debits: "0.001",
    pending_expiry: "0",
    matching_allowance_seconds: 60,
    available_grants: "3",
    estimated_seconds: "128146",
    shared_account_estimate_only: true,
  },
});
afterEach(cleanup);
it("shows exact quote-specific funds and matching benefits without promising duration", () => {
  render(
    <RemoteMachineBillingPreflight
      preflight={preflight}
      quoteProfile="remote-machine"
      quoteRate="accepted-rate"
      now={Date.parse("2099-01-01T00:00:10Z")}
    />,
  );
  expect(screen.getByText(/2.123456789012 Credits/)).toBeInTheDocument();
  expect(
    screen.getByText(/60 running seconds and 3 grant Credits/),
  ).toBeInTheDocument();
  expect(screen.getByText(/128146 seconds/)).toBeInTheDocument();
  expect(
    screen.getByText(/reserves nothing and does not guarantee/),
  ).toBeInTheDocument();
});
it.each(["other-rate", "expired"])(
  "does not show estimates from a different rate or stale observation: %s",
  (reason) => {
    render(
      <RemoteMachineBillingPreflight
        preflight={preflight}
        quoteRate={reason === "expired" ? "accepted-rate" : reason}
        now={Date.parse(
          reason === "expired"
            ? "2099-01-01T00:00:31Z"
            : "2099-01-01T00:00:10Z",
        )}
      />,
    );
    expect(screen.queryByText(/128146 seconds/)).not.toBeInTheDocument();
    expect(
      screen.getByText(/benefits and runtime estimate: unavailable/),
    ).toBeInTheDocument();
  },
);
it("keeps absent preflight unknown and rejects a reservation guarantee", () => {
  render(<RemoteMachineBillingPreflight preflight={null} />);
  expect(screen.getByText(/Unknown for this quote/)).toBeInTheDocument();
  expect(
    remoteMachineBillingPreflightSchema.safeParse({
      ...preflight,
      reservation_guarantee: true,
    }).success,
  ).toBe(false);
});
it("explains retained-volume quota demand and non-guaranteed headroom", () => {
  render(
    <RemoteMachineQuota
      quota={{
        status: "headroom_observed",
        reason: "bounded_inventory_complete",
        operation: "retained_volume_resume",
        profile_version: "remote-machine",
        additional_profiles_upper_bound: 2,
        observed_at: "2099-01-01T00:00:00Z",
        valid_until: "2099-01-01T00:00:30Z",
        capacity_guarantee: false,
        quota_usage_freshness: "controller_reconciliation_time_unknown",
      }}
    />,
  );
  expect(
    screen.getByText(/reuses the retained workspace volume/),
  ).toBeInTheDocument();
  expect(screen.getByText(/not a capacity guarantee/)).toBeInTheDocument();
});


it("does not turn unknown temporary quota demand into a Machine count", () => {
  render(<RemoteMachineQuota quota={{
    status: "unknown", reason: "ephemeral_storage_demand_unknown", operation: "create",
    profile_version: "remote-machine", additional_profiles_upper_bound: null,
    observed_at: "2099-01-01T00:00:00Z", valid_until: "2099-01-01T00:00:30Z",
    capacity_guarantee: false, quota_usage_freshness: "controller_reconciliation_time_unknown",
  }} />);
  expect(screen.getByText(/Namespace quota: unknown/)).toBeInTheDocument();
  expect(screen.queryByText(/additional fixed profiles/)).not.toBeInTheDocument();
  expect(screen.getByText(/Temporary disk capacity and per-Machine quota demand remain unknown/)).toBeInTheDocument();
});


it.each(["other-profile", "other-price"])(
  "withholds a same-rate estimate bound to %s",
  (mismatch) => {
    render(
      <RemoteMachineBillingPreflight
        preflight={{
          ...preflight,
          profile_version:
            mismatch === "other-profile" ? "fixture-other" : "remote-machine",
        }}
        quoteProfile="remote-machine"
        quoteRate="accepted-rate"
        quotePrice={mismatch === "other-price" ? "0.000041" : "0.00004"}
        now={Date.parse("2099-01-01T00:00:10Z")}
      />,
    );
    expect(screen.queryByText(/128146 seconds/)).not.toBeInTheDocument();
    expect(screen.getByText(/Unknown for this quote/)).toBeInTheDocument();
  },
);

it("compares exact price values without depending on decimal formatting", () => {
  render(
    <RemoteMachineBillingPreflight
      preflight={preflight}
      quoteProfile="remote-machine"
      quoteRate="accepted-rate"
      quotePrice="0.00004"
      now={Date.parse("2099-01-01T00:00:10Z")}
    />,
  );
  expect(screen.getByText(/128146 seconds/)).toBeInTheDocument();
});
