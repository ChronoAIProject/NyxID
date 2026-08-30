import type {
  PlatformOperationDiscoveryPricing,
  PlatformOperationPricing,
} from "@/schemas/platform-ops";

/**
 * Builders for platform-operation test fixtures.
 *
 * Tests that assert behaviour still need to construct rows by hand, but every
 * hand-written row is a chance to drift from the backend. These builders keep
 * the required-field set in one place and are typed from the schemas, so a new
 * required field is a compile error here rather than a silent mismatch in a
 * dozen literals. The shape itself is pinned by platform-ops-contract.test.ts,
 * which parses a fixture generated from the Rust serializers.
 */

export function adminPricing(
  overrides: Partial<PlatformOperationPricing> = {},
): PlatformOperationPricing {
  return {
    billable: false,
    metric: "requests",
    price_per_unit: "0",
    secondary: null,
    base_fee_per_call: null,
    display: "Free",
    lago_metric_code: "",
    sync_status: "pending",
    sync_error: null,
    ...overrides,
  };
}

export function discoveryPricing(
  overrides: Partial<PlatformOperationDiscoveryPricing> = {},
): PlatformOperationDiscoveryPricing {
  return {
    billable: false,
    metric: "requests",
    price_per_unit: "0",
    secondary: null,
    base_fee_per_call: null,
    display: "Free",
    ...overrides,
  };
}

/** Per-call price, the shape most tests want when they need a billable row. */
export function perCallPricing(
  credits: string,
  overrides: Partial<PlatformOperationPricing> = {},
): PlatformOperationPricing {
  return adminPricing({
    billable: true,
    price_per_unit: credits,
    display: `${credits} credits per call`,
    ...overrides,
  });
}
