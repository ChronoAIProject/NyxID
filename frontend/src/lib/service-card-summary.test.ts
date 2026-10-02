import { describe, expect, it } from "vitest";
import {
  connectionBillability,
  latestServiceEdit,
} from "./service-card-summary";
import { configuredBilling } from "./service-insights-compat";
import type { KeyInfo } from "@/types/keys";

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
describe("card billing configuration", () => {
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
