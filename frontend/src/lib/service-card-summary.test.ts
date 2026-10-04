import { describe, expect, it } from "vitest";
import {
  connectionBillability,
  connectionBillingCategory,
  latestServiceEdit,
} from "./service-card-summary";
import { configuredBilling } from "./service-insights-compat";
import type { KeyInfo } from "@/types/keys";
import {
  configuredPlatformPrice,
  configuredUsageCharge,
  serviceBillingConfigured,
} from "./service-billing-config";

const connection = {
  id: "personal",
  catalog_service_id: "catalog",
  catalog_service_slug: "service",
  credential_binding: "user",
  credential_type: "api_key",
  api_key_id: "key",
  auth_method: "bearer",
  is_active: true,
} as KeyInfo;
const lane = {
  metric: "requests",
  credits_per_unit: "1",
  sync_status: "synced" as const,
};
const twitter = {
  slug: "api-twitter",
  billing: {
    platform_billable: true,
    platform_charge_nyxid_credentials_only: false,
    platform_key_pricing: { ...lane, credits_per_unit: "0.05" },
    byok_pricing: null,
  },
};
const oauth = { ...connection, credential_type: "oauth2" };

describe("billing gate before credential supply — reviewed acceptance cases", () => {
  it.each([
    connection,
    { ...connection, credential_binding: "platform" as const },
    oauth,
    { ...oauth, oauth_app_source: "platform" as const },
    { ...connection, node_id: "node", credential_type: "node_managed" },
    { ...connection, is_active: false },
  ])("shows a dash for every credential on an unpriced service: %j", (row) => {
    const catalog = { slug: "llm-anthropic", billing: null };
    expect(
      connectionBillingCategory(row, configuredBilling(row, catalog), catalog),
    ).toBe("not_billable");
  });

  it.each([
    {
      row: { ...oauth, oauth_app_source: "platform" as const },
      category: "platform",
    },
    { row: { ...oauth, oauth_app_source: "byo" as const }, category: "byok" },
    { row: { ...oauth, oauth_client_id: "org-app" }, category: "byok" },
    { row: oauth, category: "unknown" },
    { row: connection, category: "byok" },
    { row: { ...connection, api_key_id: null }, category: "unknown" },
    { row: { ...connection, credential_missing: true }, category: "unknown" },
    {
      row: { ...connection, credential_binding: "platform" as const },
      category: "platform",
    },
    {
      row: { ...connection, node_id: "node", credential_type: "node_managed" },
      category: "byok",
    },
    { row: { ...oauth, node_id: "node" }, category: "unknown" },
  ])(
    "uses proven provenance on a billable service: $category %j",
    ({ row, category }) => {
      for (const is_active of [true, false]) {
        const item = { ...row, is_active };
        expect(
          connectionBillingCategory(
            item,
            configuredBilling(item, twitter),
            twitter,
          ),
        ).toBe(category);
      }
    },
  );

  it("keeps Twitter OAuth's supplier separate from its absent price lane", () => {
    const row = { ...oauth, oauth_app_source: "platform" as const };
    const bill = configuredBilling(row, twitter);
    expect(bill).toMatchObject({
      service_billing_configured: true,
      credential_supplier: "nyxid",
      credit_billing_configured: false,
      rates: [],
    });
    expect(connectionBillingCategory(row, bill, twitter)).toBe("platform");
    const legacy = configuredBilling(oauth, twitter);
    expect(legacy.credit_billing_configured).toBe(false);
    expect(
      connectionBillingCategory(
        oauth,
        { ...legacy, credential_class: "user_owned" },
        twitter,
      ),
    ).toBe("unknown");
  });

  it("uses durable OAuth selection ahead of retained app hints", () => {
    const row = {
      ...oauth,
      oauth_app_source: "platform" as const,
      oauth_client_id: "retained-app",
    };
    expect(
      connectionBillingCategory(row, configuredBilling(row, twitter)),
    ).toBe("platform");
  });

  it("does not mistake retained keys for the selected agent override", () => {
    const row = { ...connection, credential_binding: "platform" as const };
    const bill = {
      ...configuredBilling(row, twitter),
      context: "agent_key",
      credential_class: "agent_override_user_owned",
      credential_supplier: "own" as const,
    };
    expect(connectionBillingCategory(row, bill)).toBe("byok");
    expect(
      connectionBillingCategory(row, {
        ...bill,
        credential_supplier: "unknown",
      }),
    ).toBe("unknown");
    expect(
      connectionBillingCategory(row, {
        ...bill,
        service_billing_configured: false,
      }),
    ).toBe("not_billable");
  });

  it("does not label missing catalog data or restricted rows as free", () => {
    expect(
      connectionBillingCategory(connection, configuredBilling(connection)),
    ).toBe("unknown");
    expect(
      connectionBillingCategory(connection, {
        ...configuredBilling(connection, twitter),
        status: "restricted",
      }),
    ).toBe("unknown");
    const custom = {
      ...connection,
      catalog_service_id: null,
      catalog_service_slug: null,
      node_id: "node",
    };
    expect(connectionBillingCategory(custom, configuredBilling(custom))).toBe(
      "not_billable",
    );
  });

  it("retains the distinction for no-auth legacy charges", () => {
    const row = { ...connection, auth_method: "none", api_key_id: null };
    expect(
      connectionBillingCategory(row, configuredBilling(row, twitter)),
    ).toBe("not_billable");
    const legacy = { slug: "legacy", billing: { platform_billable: true } };
    expect(connectionBillingCategory(row, configuredBilling(row, legacy))).toBe(
      "platform",
    );
  });

  it("recognizes service-wide charges without requiring this connection's lane", () => {
    expect(serviceBillingConfigured(oauth, twitter)).toBe(true);
    expect(
      serviceBillingConfigured(connection, {
        slug: "chrono-llm",
        billing: null,
      }),
    ).toBe(false);
    expect(serviceBillingConfigured(connection)).toBeUndefined();
  });
});

describe("card billing configuration", () => {
  it("does not advertise a zero platform price as billable", () => {
    const platform = { ...connection, credential_binding: "platform" as const };
    const catalog = {
      slug: "zero",
      billing: { platform_key_pricing: { ...lane, credits_per_unit: "0" } },
    };
    expect(configuredUsageCharge(platform, catalog)).toBe(false);
    expect(
      connectionBillingCategory(
        platform,
        configuredBilling(platform, catalog),
        catalog,
      ),
    ).toBe("not_billable");
    expect(
      configuredUsageCharge(platform, {
        slug: "charged",
        billing: { platform_key_pricing: lane },
      }),
    ).toBe(true);
  });
  it("keeps a supplied key BYOK when the catalog also offers platform pricing", () => {
    const catalog = {
      slug: "llm-deepseek",
      billing: { platform_key_pricing: lane },
    };
    expect(configuredPlatformPrice(connection, catalog)).toEqual(lane);
    const bill = configuredBilling(connection, catalog);
    expect(connectionBillingCategory(connection, bill, catalog)).toBe("byok");
    expect(bill.credential_label).toBe("Your API key (BYOK)");
    expect(bill.provider_billing).toBe("separate_provider_account");
    expect(bill.rates).toEqual([]);
  });
  it("counts only the billable connection across five personal apps and one platform connection, including disabled rows", () => {
    const catalog = {
      slug: "twitter",
      billing: { platform_key_pricing: lane, byok_pricing: null },
    };
    const rows = [
      ...Array.from({ length: 5 }, (_, i) => ({
        ...connection,
        id: `app-${i}`,
      })),
      {
        ...connection,
        id: "platform",
        credential_binding: "platform" as const,
        is_active: false,
      },
    ];
    expect(
      rows.map((row) =>
        connectionBillability(row, configuredBilling(row, catalog), catalog),
      ),
    ).toEqual([false, false, false, false, false, true]);
  });
  it("checks additional usage prices even when the primary price is zero", () => {
    const catalog = {
      slug: "twitter",
      byok_pricing: {
        ...lane,
        credits_per_unit: "0",
        components: [
          { ...lane, metric: "images", credits_per_unit: "0.000000000001" },
        ],
      },
    };
    expect(
      connectionBillability(
        connection,
        configuredBilling(connection, catalog),
        catalog,
      ),
    ).toBe(true);
    const zero = {
      slug: "twitter",
      byok_pricing: { ...lane, credits_per_unit: "0.000" },
    };
    expect(
      connectionBillability(
        connection,
        configuredBilling(connection, zero),
        zero,
      ),
    ).toBe(false);
  });
  it("does not equate missing data or a caller's free usage with a nonbillable service", () => {
    const bill = configuredBilling(connection);
    expect(connectionBillability(connection, bill)).toBeUndefined();
    expect(
      connectionBillability(connection, {
        ...bill,
        charge_status: "not_charged",
      }),
    ).toBeUndefined();
    expect(
      connectionBillability(connection, {
        ...bill,
        status: "restricted",
        credit_billing_configured: true,
      }),
    ).toBeUndefined();
  });
  it("keeps legacy billing visible until a zero service price has synced", () => {
    for (const sync_status of ["pending", "synced"] as const) {
      const catalog = {
        slug: "twitter",
        billing: {
          platform_billable: true,
          platform_pricing: { credits_per_unit: "0", sync_status },
        },
      };
      expect(
        connectionBillability(
          connection,
          configuredBilling(connection, catalog),
          catalog,
        ),
      ).toBe(sync_status === "pending");
    }
  });
  it("uses the backend's configured flag even when execution is unavailable or covered for the caller", () => {
    const bill = configuredBilling(connection);
    expect(
      connectionBillability(
        { ...connection, is_active: false },
        { ...bill, status: "unavailable", credit_billing_configured: true },
      ),
    ).toBe(true);
    expect(
      connectionBillability(connection, {
        ...bill,
        charge_status: "not_charged",
        credit_billing_configured: true,
      }),
    ).toBe(true);
    expect(
      connectionBillability(connection, {
        ...bill,
        credit_billing_configured: false,
      }),
    ).toBe(false);
  });
  it("does not assume the owner of an OAuth developer app", () => {
    const oauth = { ...connection, credential_type: "oauth2" };
    const catalog = {
      slug: "twitter",
      billing: {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: true,
      },
    };
    expect(
      connectionBillability(oauth, configuredBilling(oauth, catalog), catalog),
    ).toBeUndefined();
    expect(
      connectionBillability(
        connection,
        configuredBilling(connection, catalog),
        catalog,
      ),
    ).toBe(false);
  });
});

it("selects the most recent recorded edit without substituting credential use or creation", () => {
  const actor = {
    kind: "person",
    id: "person",
    name: "Calvin",
    person_id: "person",
    api_key_id: null,
    app_id: null,
  };
  const edit = {
    at: "2026-10-01T10:00:00Z",
    action: "updated",
    change_group_id: "edit",
    actor,
  };
  const older = {
    ...connection,
    authorship: { created_by: null, last_change: edit },
  };
  const newer = {
    ...older,
    id: "newer",
    authorship: {
      created_by: null,
      last_change: { ...edit, at: "2026-10-02T10:00:00Z" },
    },
  };
  const created = {
    ...connection,
    id: "created",
    created_at: "2026-10-03T10:00:00Z",
    last_used_at: "2026-10-04T10:00:00Z",
    authorship: {
      created_by: { ...edit, at: "2026-10-03T10:00:00Z" },
      last_change: null,
    },
  };
  expect(latestServiceEdit([newer, older, created])?.connection.id).toBe(
    "newer",
  );
  expect(latestServiceEdit([created])).toBeUndefined();
});
