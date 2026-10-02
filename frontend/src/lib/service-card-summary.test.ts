import { describe, expect, it } from "vitest";
import {
  connectionBillability,
  connectionBillingCategory,
  latestServiceEdit,
} from "./service-card-summary";
import { configuredBilling } from "./service-insights-compat";
import type { KeyInfo } from "@/types/keys";
import { configuredPlatformPrice } from "./service-billing-config";

const connection = {
  id: "personal",
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
describe("card billing categories", () => {
  it("identifies NyxID's shared OAuth app without calling a personal OAuth login BYOK", () => {
    const oauth = { ...connection, credential_type: "oauth2" };
    const bill = configuredBilling(oauth);
    expect(connectionBillingCategory(oauth, bill)).toBe("unknown");
    expect(
      connectionBillingCategory(oauth, {
        ...bill,
        credential_class: "nyxid_platform_oauth_app",
      }),
    ).toBe("platform");
    expect(
      connectionBillingCategory(oauth, {
        ...bill,
        credential_class: "user_owned",
      }),
    ).toBe("byok");
  });
  it("keeps a supplied key BYOK when NyxID charges apply or credits cover its usage", () => {
    expect(
      connectionBillingCategory(connection, {
        ...configuredBilling(connection),
        credit_billing_configured: true,
        charge_status: "not_charged",
      }),
    ).toBe("byok");
  });
  it("does not call missing credentials, restricted data, or legacy charges not billable", () => {
    const noAuth = { ...connection, auth_method: "none", api_key_id: null };
    const bill = configuredBilling(noAuth);
    expect(connectionBillingCategory(noAuth, bill)).toBe("unknown");
    expect(
      connectionBillingCategory(noAuth, {
        ...bill,
        credit_billing_configured: false,
      }),
    ).toBe("not_billable");
    expect(
      connectionBillingCategory(noAuth, {
        ...bill,
        credit_billing_configured: true,
      }),
    ).toBe("unknown");
    expect(
      connectionBillingCategory(connection, {
        ...bill,
        status: "restricted",
        credential_class: "user_owned",
      }),
    ).toBe("unknown");
    expect(
      connectionBillingCategory({ ...connection, credential_missing: true }),
    ).toBe("unknown");
  });
  it("uses the selected agent override instead of the platform connection default", () => {
    const platform = { ...connection, credential_binding: "platform" as const };
    const bill = { ...configuredBilling(platform), context: "agent_key" };
    expect(
      connectionBillingCategory(platform, {
        ...bill,
        credential_class: "agent_override_user_owned",
      }),
    ).toBe("byok");
    expect(connectionBillingCategory(platform, bill)).toBe("unknown");
  });
});

describe("card billing configuration", () => {
  it("reads platform prices from service billing even when the connection uses its own credential", () => {
    const catalog = {
      slug: "llm-deepseek",
      billing: { platform_key_pricing: lane },
    };
    expect(configuredPlatformPrice(connection, catalog)).toEqual(lane);
    const bill = configuredBilling(connection, catalog);
    expect(connectionBillingCategory(connection, bill, catalog)).toBe(
      "unknown",
    );
    expect(bill.credential_label).toBe("Stored API key · supplier unverified");
    expect(bill.provider_billing).toBe("unknown");
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
