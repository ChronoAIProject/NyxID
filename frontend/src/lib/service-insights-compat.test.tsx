import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  vi,
  type MockInstance,
} from "vitest";
import type { ReactNode } from "react";
import { api, ApiError } from "@/lib/api-client";
import { useServiceInsights } from "@/hooks/use-service-insights";
import {
  loadConfiguredServiceInsights,
  configuredBilling,
} from "./service-insights-compat";
import type { KeyInfo } from "@/types/keys";

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "person" } }),
}));
const personal = {
  id: "personal",
  api_key_id: "stored-api-key",
  label: "OpenAI",
  catalog_service_slug: "openai",
  credential_source: { type: "personal" },
  credential_binding: "user",
  auth_method: "bearer",
  credential_type: "api_key",
  is_active: true,
} as KeyInfo;
const org = {
  ...personal,
  id: "org-service",
  credential_source: {
    type: "org",
    org_id: "org",
    org_name: "ChronoAI",
    role: "admin",
    allowed: true,
  },
} as KeyInfo;
const key = {
  id: "key",
  name: "Codex",
  platform: "codex",
  purpose: "general",
  is_active: true,
  expires_at: null,
  scopes: "proxy",
  allow_all_services: false,
  allowed_service_ids: ["personal"],
  bindings_count: 0,
};
const responses = new Map<string, unknown>();
const unavailable = (status = 404) =>
  new ApiError(status, {
    error: "unavailable",
    error_code: status,
    message: "Unavailable",
  });
let get: MockInstance<typeof api.get>;
beforeEach(() => {
  responses.clear();
  responses.set("/api-keys", { keys: [key] });
  responses.set("/orgs", { orgs: [{ id: "org", your_role: "admin" }] });
  responses.set("/api-keys?org_id=org", {
    keys: [{ ...key, id: "org-key", allow_all_services: true }],
  });
  responses.set("/catalog?include_all=true", {
    entries: [
      {
        slug: "openai",
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.12",
          sync_status: "synced",
        },
      },
    ],
  });
  get = vi.spyOn(api, "get").mockImplementation(async (path) => {
    const response = responses.get(path);
    if (response instanceof Error) throw response;
    if (!response) throw unavailable();
    return response;
  });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
function mount(connections: KeyInfo[] = [personal]) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return renderHook(() => useServiceInsights(connections), {
    wrapper: ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
}

describe("deployed service insight compatibility", () => {
  it("uses the connection's own OAuth app and honors platform-only charge exclusions", () => {
    const bill = configuredBilling(
      {
        ...org,
        credential_type: "oauth2",
        oauth_client_id: "organization-app",
      },
      {
        slug: "twitter",
        billing: {
          platform_charge_nyxid_credentials_only: true,
          byok_pricing: {
            metric: "requests",
            credits_per_unit: "0.01",
            sync_status: "synced",
          },
          platform_key_pricing: {
            metric: "requests",
            credits_per_unit: "0.05",
            sync_status: "synced",
          },
        },
      },
    );
    expect(bill).toMatchObject({
      credential_class: "user_owned",
      credential_label: "Organization OAuth app (BYOK)",
      provider_billing: "separate_provider_account",
      credit_billing_configured: false,
      rates: [],
    });
    expect(bill.notes.join(" ")).not.toContain("does not report whether");
  });

  it("selects a supplied key's own fee lane when both credential classes are priced", () => {
    const bill = configuredBilling(personal, {
      slug: "openai",
      billing: {
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.01",
          sync_status: "synced",
        },
        platform_key_pricing: {
          metric: "requests",
          credits_per_unit: "0.05",
          sync_status: "synced",
        },
      },
    });
    expect(bill).toMatchObject({
      credential_class: "user_owned",
      credential_label: "Your API key (BYOK)",
      credit_billing_configured: true,
      rates: [{ credits_per_unit: "0.01" }],
    });
    expect(bill.rates).toHaveLength(1);
  });

  it("treats omitted catalog billing as unpriced while retaining OAuth provenance separately", async () => {
    responses.set("/catalog?include_all=true", {
      entries: [{ slug: "openai" }],
    });
    const [item] = await loadConfiguredServiceInsights(
      [{ ...personal, credential_type: "oauth2" }],
      "person",
    );
    expect(item!.billing).toMatchObject({
      credit_billing_configured: false,
      charge_status: "not_charged",
      credential_label: "Connected account · app unverified",
      rates: [],
    });
    expect(item!.billing!.notes.join(" ")).not.toContain(
      "does not mean usage is free",
    );
  });

  it.each([403, 500, "missing"] as const)(
    "does not call a catalog service unpriced after a %s catalog lookup",
    async (failure) => {
      responses.set(
        "/catalog?include_all=true",
        failure === "missing" ? { entries: [] } : unavailable(failure),
      );
      const [catalogService, customService] =
        await loadConfiguredServiceInsights(
          [
            personal,
            {
              ...personal,
              id: "custom",
              source: "custom",
              catalog_service_id: null,
              catalog_service_slug: null,
            },
          ],
          "person",
        );
      expect(
        catalogService!.billing!.credit_billing_configured,
      ).toBeUndefined();
      expect(catalogService!.billing!.charge_status).toBe("conditional");
      expect(customService!.billing!.credit_billing_configured).toBe(false);
      expect(customService!.billing!.charge_status).toBe("not_charged");
    },
  );
  it("does not classify an OAuth login as a supplied developer app", () => {
    const bill = configuredBilling({
      ...personal,
      credential_type: "oauth2",
      api_key_id: "oauth-token",
    });
    expect(bill.provider_billing).toBe("unknown");
    expect(bill.credential_label).toBe("Connected account · app unverified");
    expect(bill.credential_label).not.toContain("BYOK");
  });

  it("reads legacy catalog credit billing without claiming the caller has been charged", async () => {
    responses.set("/catalog?include_all=true", {
      entries: [
        {
          slug: "openai",
          billing: {
            platform_billable: true,
            platform_metric: "requests",
            platform_pricing: {
              credits_per_unit: "0.2",
              sync_status: "synced",
            },
          },
        },
      ],
    });
    const [item] = await loadConfiguredServiceInsights([personal], "person");
    expect(item!.billing).toMatchObject({
      credit_billing_configured: true,
      charge_status: "conditional",
      rates: [{ credits_per_unit: "0.2", metric: "requests" }],
    });
  });

  it("does not apply legacy pricing over a different credential lane", () => {
    const bill = configuredBilling(personal, {
      slug: "openai",
      billing: {
        platform_billable: true,
        platform_metric: "requests",
        platform_key_pricing: {
          metric: "requests",
          credits_per_unit: "0.5",
          sync_status: "synced",
        },
      },
    });
    expect(bill.credit_billing_configured).toBe(false);
    expect(bill.rates).toEqual([]);
    expect(bill.charge_status).toBe("not_charged");
  });

  it("does not advertise platform-only charges on a known user-supplied API key", () => {
    const bill = configuredBilling(personal, {
      slug: "openai",
      billing: {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: true,
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.5",
          sync_status: "synced",
        },
      },
    });
    expect(bill.credit_billing_configured).toBe(false);
    expect(bill.rates).toEqual([]);
  });

  it("preserves unreported plan rates without inventing a numeric price", () => {
    const bill = configuredBilling(personal, {
      slug: "openai",
      billing: {
        platform_billable: true,
        platform_metric: "requests",
      },
    });
    expect(bill.credit_billing_configured).toBe(true);
    expect(bill.rates[0]?.credits_per_unit).toBeNull();
  });

  it("accepts the deployed key list's omitted expiry and zero binding count", async () => {
    const {
      expires_at: _expiry,
      bindings_count: _count,
      ...withoutOptionalFields
    } = key;
    void _expiry;
    void _count;
    responses.set("/api-keys", { keys: [withoutOptionalFields] });
    const [insight] = await loadConfiguredServiceInsights([personal], "person");
    expect(insight!.usage!.access.incomplete).toBe(false);
    expect(insight!.usage!.access.keys).toMatchObject([
      { id: "key", name: "Codex", credential_override: false },
    ]);
  });
  it("uses live scope and catalog metadata when the insights route is absent, without inventing recorded use or settled billing", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.status).toBe("ready"));
    const insight = result.current.connections.get(personal.id)!;
    expect(insight.usage?.access.keys.map((item) => item.name)).toEqual([
      "Codex",
    ]);
    expect(insight.usage?.access.basis).toBe("configuration");
    expect(insight.usage?.activity.tracking).toBe("unavailable");
    expect(insight.billing).toMatchObject({
      status: "conditional",
      context: "configuration",
      account: null,
      payer_rule: "Your personal account",
      rates: [{ credits_per_unit: "0.12", sync_status: "synced" }],
    });
  });
  it.each([403, 500])(
    "does not fall back on an insights %s failure",
    async (status) => {
      responses.set("/service-insights?ids=personal", unavailable(status));
      const { result } = mount();
      await waitFor(() =>
        expect(result.current.status).toBe(
          status === 403 ? "restricted" : "error",
        ),
      );
      expect(get).toHaveBeenCalledTimes(1);
      expect(result.current.connections.size).toBe(0);
    },
  );
  it("clears prior insight data when a refresh loses permission", async () => {
    const { result } = mount();
    await waitFor(() => expect(result.current.connections.size).toBe(1));
    responses.set("/service-insights?ids=personal", unavailable(403));
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.status).toBe("restricted"));
    expect(result.current.connections.size).toBe(0);
  });
  it("does not hide an authorization failure in a second batch behind an unsupported first batch", async () => {
    get
      .mockRejectedValueOnce(unavailable())
      .mockRejectedValueOnce(unavailable(403));
    const { result } = mount(
      Array.from({ length: 101 }, (_, index) => ({
        ...personal,
        id: `connection-${index}`,
      })),
    );
    await waitFor(() => expect(result.current.status).toBe("restricted"));
    expect(get).toHaveBeenCalledTimes(2);
    expect(result.current.connections.size).toBe(0);
  });
  it("matches exact connection ids, scope, active keys, expiry and purpose", async () => {
    responses.set("/api-keys", {
      keys: [
        key,
        { ...key, id: "catalog-only", allowed_service_ids: ["openai"] },
        { ...key, id: "expired", expires_at: "2000-01-01T00:00:00Z" },
        { ...key, id: "inactive", is_active: false },
        { ...key, id: "read-only", scopes: "account:read" },
        { ...key, id: "scheduled", purpose: "scheduled_invocation" },
        { ...key, id: "unknown-purpose", purpose: undefined },
        { ...key, id: "wide", scopes: "llm:proxy", allow_all_services: true },
      ],
    });
    const [a, duplicate] = await loadConfiguredServiceInsights(
      [personal, { ...personal, id: "duplicate" }],
      "person",
    );
    expect(a!.usage!.access.keys.map((item) => item.id)).toEqual([
      "key",
      "wide",
    ]);
    expect(duplicate!.usage!.access.keys.map((item) => item.id)).toEqual([
      "wide",
    ]);
    expect(a!.usage!.access.incomplete).toBe(true);
  });
  it("keeps organization keys within their owner and personal keys within permitted org scope", async () => {
    responses.set("/api-keys", {
      keys: [{ ...key, allow_all_services: true }],
    });
    const [a, b, c] = await loadConfiguredServiceInsights(
      [
        personal,
        org,
        {
          ...org,
          id: "excluded",
          credential_source: {
            ...org.credential_source!,
            type: "org",
            org_id: "other",
            org_name: "Other",
            role: "viewer",
            allowed: false,
          },
        },
      ],
      "person",
    );
    expect(a!.usage!.access.keys.map((item) => item.id)).toEqual(["key"]);
    expect(b!.usage!.access.keys.map((item) => item.id)).toEqual([
      "key",
      "org-key",
    ]);
    expect(c!.usage!.access.keys).toEqual([]);
    expect(get).not.toHaveBeenCalledWith("/api-keys?org_id=other");
  });
  it("matches overrides by both key and exact connection, and preserves unknown overrides on failures", async () => {
    responses.set("/api-keys", {
      keys: [
        { ...key, id: "bound", allow_all_services: true, bindings_count: 1 },
        { ...key, id: "unknown", bindings_count: 1 },
      ],
    });
    responses.set("/api-keys/bound/bindings", {
      bindings: [{ api_key_id: "bound", user_service_id: "personal" }],
    });
    const [a, b] = await loadConfiguredServiceInsights(
      [personal, { ...personal, id: "duplicate" }],
      "person",
    );
    expect(
      a!.usage!.access.keys.map((item) => item.credential_override),
    ).toEqual([true, null]);
    expect(
      b!.usage!.access.keys.map((item) => item.credential_override),
    ).toEqual([false]);
  });
  it("marks an unavailable key inventory instead of reporting zero accessible keys", async () => {
    responses.set("/api-keys", unavailable(403));
    const [a] = await loadConfiguredServiceInsights([personal], "person");
    expect(a!.usage!.access.visibility).toBe("unavailable");
    expect(a!.usage!.access.incomplete).toBe(true);
  });
  it("keeps platform credential supply separate from who pays and flags unsynced prices", () => {
    const bill = configuredBilling({
      ...org,
      credential_binding: "platform",
      platform_key_pricing: {
        metric: "requests",
        credits_per_unit: "0.01",
        sync_status: "pending",
        components: [
          {
            metric: "output_tokens",
            credits_per_unit: "0.000000000012",
            sync_status: "failed",
          },
        ],
      },
    });
    expect(bill.payer_rule).toBe("Acting user's personal account");
    expect(bill.account).toBeNull();
    expect(bill.rates.map((rate) => rate.sync_status)).toEqual([
      "pending",
      "failed",
    ]);
    expect(bill.charge_status).toBe("conditional");
    expect(configuredBilling(org).payer_rule).toBe("ChronoAI · organization");
    expect(configuredBilling(personal).charge_status).not.toBe("not_charged");
  });
});
