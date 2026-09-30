import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  render,
  screen,
  within,
  cleanup,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { KeyInfo } from "@/types/keys";
import type { ServiceInsight } from "@/schemas/service-insights";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import {
  billingAccountLabel,
  billingModelLabel,
  summarizeBilling,
  summarizeBillingModel,
} from "@/lib/service-insights";
import { configuredBilling } from "@/lib/service-insights-compat";
import { api } from "@/lib/api-client";
import { ServiceConnectionTable } from "./service-connection-table";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    ...props
  }: {
    children: ReactNode;
    to: string;
    params?: { keyId: string };
    "aria-label"?: string;
  }) => (
    <a
      href={to.replace("$keyId", params?.keyId ?? "")}
      aria-label={props["aria-label"]}
    >
      {children}
    </a>
  ),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: { id: string } }) => unknown) =>
    selector({ user: { id: "person" } }),
}));
vi.mock("./service-history", () => ({
  ServiceHistory: () => <div>Permitted service history</div>,
}));

const connection = {
  id: "connection-a",
  label: "Team OpenAI",
  slug: "openai-team",
  catalog_service_id: "openai",
  catalog_service_slug: "openai",
  catalog_service_name: "OpenAI",
  service_type: "http",
  is_active: true,
  status: "active",
  auth_method: "bearer",
  credential_type: "api_key",
  node_id: null,
  auto_connected: false,
  can_edit_configuration: false,
  credential_binding: "platform",
  credential_source: {
    type: "org",
    org_id: "org",
    org_name: "ChronoAI",
    role: "member",
    allowed: true,
  },
  endpoint_url: "https://private.example.test",
  endpoint_id: "endpoint",
  auth_key_name: "Authorization",
  node_priority: 0,
  expires_at: null,
  last_used_at: null,
  error_message: null,
  ssh_host: null,
  ssh_port: null,
  ssh_ca_public_key: null,
  ssh_allowed_principals: null,
  ssh_certificate_ttl_minutes: null,
  created_at: "2026-09-01T00:00:00Z",
  source_app_name: "Provisioning app",
  ws_frame_injections: [],
} as KeyInfo;
const insight: ServiceInsight = {
  service_id: connection.id,
  billing: {
    status: "resolved",
    credential_class: "nyxid_managed_master",
    credential_label: "NyxID credential",
    account: { id: "person", kind: "personal", name: "Your personal account" },
    charge_status: "usage_based",
    rates: [
      {
        layer: "platform",
        metric: "requests",
        credits_per_unit: "0.25",
        currency: "credits",
        source: "credential_lane",
      },
    ],
    provider_billing: "nyxid_credential",
    context: "for_you",
    notes: [],
  },
  usage: {
    access: {
      visibility: "own_keys",
      keys: [
        {
          id: "agent-1",
          name: "Codex CI",
          platform: "codex",
          owner_id: "person",
          permission: "selected_service",
          credential_override: false,
        },
        {
          id: "agent-2",
          name: "Unused worker",
          platform: null,
          owner_id: "person",
          permission: "all_services",
          credential_override: true,
        },
      ],
      truncated: false,
    },
    activity: {
      visibility: "own_requests",
      period_days: 30,
      tracking: "partial",
      request_count: 1,
      requests: [
        {
          id: "event-1",
          execution_id: "execution-1",
          caller: {
            id: "agent-1",
            kind: "agent_key",
            name: "Codex CI",
            app_id: "app",
            app_name: "Release app",
          },
          occurred_at: "2026-09-29T00:00:00Z",
          outcome: "response_received",
          response_status: 200,
          source: { kind: "platform", owner_id: "org" },
        },
      ],
      truncated: false,
    },
  },
};
function mount(
  value: ServiceInsight | undefined = insight,
  status: ServiceInsightsState["status"] = "ready",
) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ServiceConnectionTable
        connections={[connection]}
        serviceName="OpenAI"
        insights={{
          connections: new Map(value ? [[connection.id, value]] : []),
          status,
          refresh: vi.fn(),
        }}
      />
    </QueryClientProvider>,
  );
}
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("service card billing and caller details", () => {
  it("shows the recorded layer even when today's connection binding is different", () => {
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        activity: {
          ...insight.usage!.activity,
          requests: [
            {
              ...insight.usage!.activity.requests[0]!,
              source: { kind: "org", owner_id: "org" },
            },
          ],
        },
      },
    });
    expect(screen.getByTitle(/^Organization · ChronoAI · /)).toBeVisible();
    expect(screen.getByText(/Codex CI · Release app/)).toBeVisible();
    expect(screen.getByText("Personal account")).toBeVisible();
  });
  it("does not present incomplete key inventory as zero keys", () => {
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        access: { ...insight.usage!.access, keys: [], incomplete: true },
      },
    });
    expect(screen.getByText("Key access incomplete")).toBeVisible();
    expect(screen.queryByText("0 agent keys")).not.toBeInTheDocument();
  });
  it("shows the separately recorded last use even when all three recent requests were denied", () => {
    const last = insight.usage!.activity.requests[0]!;
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        activity: {
          ...insight.usage!.activity,
          last_used: last,
          requests: Array.from({ length: 3 }, (_, index) => ({
            ...last,
            id: `denied-${index}`,
            outcome: "denied",
            response_status: 403,
            source: null,
          })),
        },
      },
    });
    expect(screen.getByTitle(/^Platform · /)).toBeVisible();
    expect(screen.getByText(/Codex CI · Release app/)).toBeVisible();
  });
  it("leaves legacy request layers unknown even when the current binding is platform", () => {
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        activity: {
          ...insight.usage!.activity,
          requests: [{ ...insight.usage!.activity.requests[0]!, source: null }],
        },
      },
    });
    expect(screen.getByTitle(/^Layer not recorded · /)).toBeVisible();
    expect(screen.queryByTitle(/^Platform · /)).not.toBeInTheDocument();
  });
  it("does not infer the last used layer from credential timestamps or rejected requests", () => {
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        activity: {
          ...insight.usage!.activity,
          requests: [
            {
              ...insight.usage!.activity.requests[0]!,
              outcome: "denied",
              response_status: 403,
            },
          ],
        },
      },
    });
    expect(screen.getByText("Not recorded")).toBeVisible();
    expect(
      screen.queryByText("Platform · openai-team"),
    ).not.toBeInTheDocument();
  });
  it("shows configured key access and the billing flow on older servers without claiming recorded use", async () => {
    const user = userEvent.setup();
    mount({
      ...insight,
      billing: configuredBilling({
        ...connection,
        platform_key_pricing: {
          metric: "requests",
          credits_per_unit: "0.05",
          sync_status: "pending",
        },
      }),
      usage: {
        access: {
          ...insight.usage!.access,
          basis: "configuration",
          keys: [
            { ...insight.usage!.access.keys[0]!, credential_override: null },
          ],
        },
        activity: {
          ...insight.usage!.activity,
          tracking: "unavailable",
          visibility: "unavailable",
          request_count: 0,
          requests: [],
        },
      },
    });
    expect(screen.getByTitle(/Configured scope[\s\S]*Codex CI/)).toBeVisible();
    expect(screen.getByText("Acting user's personal account")).toBeVisible();
    expect(screen.queryByText(/No recorded requests/)).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    const panel = screen.getByRole("region", {
      name: "Billing for Team OpenAI",
    });
    expect(within(panel).getByText("Billing flow")).toBeVisible();
    expect(within(panel).getByText("Expected payer")).toBeVisible();
    expect(within(panel).getByText("Pending")).toBeVisible();
    expect(within(panel).queryByRole("combobox")).not.toBeInTheDocument();
    expect(
      within(panel).getByRole("table", { name: "Configured NyxID rates" }),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Recent requests for Team OpenAI" }),
    );
    expect(screen.getByText("Request attribution unavailable")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Agent key access for Team OpenAI" }),
    );
    expect(screen.getByText("Agent keys in scope")).toBeVisible();
    expect(screen.getByText("Override not reported")).toBeVisible();
  });
  it("exposes payer, credential and actual last caller directly in the expanded table", () => {
    mount();
    expect(screen.getByRole("columnheader", { name: "Billing" })).toBeVisible();
    expect(screen.getByText("Personal account")).toBeVisible();
    expect(screen.getByText(/NyxID credential/)).toBeVisible();
    expect(
      within(
        screen.getByRole("button", { name: "Recent requests for Team OpenAI" }),
      ).getByText(/Codex CI/),
    ).toBeVisible();
    expect(screen.getByText("NyxID-managed")).toBeVisible();
    expect(screen.getByText("NyxID fee: 0.25 credits / request")).toBeVisible();
    expect(screen.getByText(/· 1 override$/)).toBeVisible();
    expect(screen.getByTitle(/^Your keys with access/)).toBeVisible();
    expect(screen.queryByText("Provisioning app")).not.toBeInTheDocument();
    expect(
      screen.queryByText("https://private.example.test"),
    ).not.toBeInTheDocument();
  });
  it("opens rates inside the table and replaces them with access and request attribution", async () => {
    const user = userEvent.setup();
    mount();
    await user.click(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    expect(
      screen.getByRole("table", { name: "Applicable NyxID rates" }),
    ).toBeVisible();
    expect(
      screen.getByText(
        /NyxID supplies the credential and handles provider billing/,
      ),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Agent key access for Team OpenAI" }),
    );
    expect(
      screen.queryByRole("table", { name: "Applicable NyxID rates" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Codex CI" })).toHaveAttribute(
      "href",
      "/keys/api-key/agent-1",
    );
    expect(screen.getByText("Unused worker")).toBeVisible();
    expect(screen.getByText("Credential override")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Recent requests for Team OpenAI" }),
    );
    const requests = screen.getByRole("table", {
      name: "Recent connection requests",
    });
    expect(within(requests).getByText("Release app")).toBeVisible();
    expect(within(requests).getByText("Agent key")).toBeVisible();
    expect(screen.queryByText("Unused worker")).not.toBeInTheDocument();
    expect(screen.getByText(/Tracking is partial/)).toBeVisible();
  });
  it("keeps history available to a reader while private configuration stays hidden", async () => {
    mount();
    await userEvent.click(
      screen.getByRole("button", {
        name: "History for Team OpenAI (ChronoAI)",
      }),
    );
    expect(screen.getByText("Permitted service history")).toBeVisible();
    expect(
      screen.queryByRole("link", { name: /Configure/ }),
    ).not.toBeInTheDocument();
  });
  it("resolves the selected agent key's payer in place without retaining the default payer while loading", async () => {
    const user = userEvent.setup();
    let resolvePreview!: (value: unknown) => void;
    const request = vi.spyOn(api, "get").mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvePreview = resolve;
        }),
    );
    mount();
    await user.click(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    const panel = screen.getByRole("region", {
      name: "Billing for Team OpenAI",
    });
    await user.click(
      within(panel).getByRole("combobox", { name: "Preview billing for" }),
    );
    await user.click(
      screen.getByRole("option", { name: "Codex CI · agent key" }),
    );
    await waitFor(() =>
      expect(request).toHaveBeenCalledWith(
        "/service-insights?ids=connection-a&api_key_id=agent-1",
      ),
    );
    expect(
      within(panel).queryByText("Personal account"),
    ).not.toBeInTheDocument();
    expect(within(panel).getByText(/Loading billing/)).toBeVisible();
    resolvePreview({
      connections: [
        {
          ...insight,
          billing: {
            ...insight.billing!,
            account: { id: "org", name: "ChronoAI", kind: "organization" },
            context: "agent_key",
          },
        },
      ],
    });
    expect(
      await within(panel).findByText("ChronoAI · organization"),
    ).toBeVisible();
    await user.click(
      within(panel).getByRole("combobox", { name: "Preview billing for" }),
    );
    await user.click(
      screen.getByRole("option", { name: "You · connection default" }),
    );
    expect(within(panel).getByText("Personal account")).toBeVisible();
    expect(
      within(panel).queryByText("ChronoAI · organization"),
    ).not.toBeInTheDocument();
  });
  it("shows server compatibility failures instead of claiming free service or no usage", async () => {
    mount({ ...insight, billing: null, usage: null }, "unavailable");
    expect(screen.getByText("Billing not reported")).toBeVisible();
    expect(screen.getByText("Not recorded")).toBeVisible();
    expect(screen.queryByText(/free|never used/i)).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    expect(screen.getByText(/This server does not provide/)).toBeVisible();
  });
  it.each([
    ["restricted", "History restricted", "Billing restricted"],
    ["error", "History couldn't load", "Billing couldn't load"],
    ["loading", "Loading…", "Loading billing…"],
  ] as const)(
    "distinguishes %s insights from an empty history",
    (status, activity, billing) => {
      mount({ ...insight, billing: null, usage: null }, status);
      expect(screen.getByText(activity)).toBeVisible();
      expect(screen.getByText(billing)).toBeVisible();
      expect(
        screen.queryByText(/No recorded requests/),
      ).not.toBeInTheDocument();
    },
  );
  it("labels an empty captured period without claiming that the service was never used", () => {
    mount({
      ...insight,
      usage: {
        ...insight.usage!,
        activity: {
          ...insight.usage!.activity,
          requests: [],
          request_count: 0,
        },
      },
    });
    expect(
      screen.getByTitle(/No recorded use with exact connection attribution/),
    ).toBeVisible();
    expect(screen.getByText("Not recorded")).toBeVisible();
    expect(screen.queryByText("Activity not reported")).not.toBeInTheDocument();
  });
  it("does not merge different payer accounts or hide a restricted connection in the group summary", () => {
    const org = {
      ...insight,
      billing: {
        ...insight.billing!,
        account: { id: "org", name: "ChronoAI", kind: "organization" as const },
      },
    };
    expect(summarizeBilling([insight, org])).toBe("Varies by connection");
    expect(
      summarizeBilling([
        insight,
        { ...org, billing: { ...org.billing, status: "unavailable" } },
      ]),
    ).toBe("Varies by connection");
    expect(summarizeBilling([insight, undefined])).toBe("Billing not reported");
    expect(
      billingAccountLabel({
        ...insight.billing!,
        status: "restricted",
        charge_status: "restricted",
      }),
    ).toBe("Billing restricted");
    expect(
      billingAccountLabel({ ...insight.billing!, status: "unavailable" }),
    ).toBe("Billing unavailable");
  });

  it("shows BYOK with separate NyxID fees even when the payer is personal", async () => {
    const user = userEvent.setup();
    mount({
      ...insight,
      billing: {
        ...insight.billing!,
        provider_billing: "separate_provider_account",
      },
    });
    const cell = within(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    expect(cell.getByText("BYOK")).toBeVisible();
    expect(cell.getByText("Personal account")).toBeVisible();
    expect(cell.getByText("NyxID fee: 0.25 credits / request")).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    expect(
      within(
        screen.getByRole("region", { name: "Billing for Team OpenAI" }),
      ).getByText(/Any NyxID service fees are additional/),
    ).toBeVisible();
  });

  it("keeps the NyxID-managed model when the service has no NyxID charge", () => {
    mount({
      ...insight,
      billing: { ...insight.billing!, charge_status: "not_charged", rates: [] },
    });
    const cell = within(
      screen.getByRole("button", { name: "Billing for Team OpenAI" }),
    );
    expect(cell.getByText("NyxID-managed")).toBeVisible();
    expect(cell.getByText("No NyxID charge")).toBeVisible();
    expect(cell.queryByText("BYOK")).not.toBeInTheDocument();
  });

  it("keeps unknown, restricted, and missing models explicit in group summaries", () => {
    const byok = {
      ...insight,
      billing: {
        ...insight.billing!,
        provider_billing: "separate_provider_account" as const,
      },
    };
    expect(summarizeBillingModel("ready", [insight, byok])).toBe(
      "BYOK + NyxID",
    );
    expect(summarizeBillingModel("ready", [byok, undefined])).toBe(
      "Billing partly reported",
    );
    expect(summarizeBillingModel("restricted", [byok])).toBe(
      "Billing restricted",
    );
    expect(
      billingModelLabel({ ...insight.billing!, provider_billing: "unknown" }),
    ).toBe("Billing model unknown");
    expect(
      billingModelLabel({ ...insight.billing!, status: "restricted" }),
    ).toBe("Billing restricted");
    expect(
      billingModelLabel({ ...insight.billing!, status: "unavailable" }),
    ).toBe("Billing unavailable");
    expect(
      billingModelLabel({
        ...insight.billing!,
        provider_billing: "no_credential",
      }),
    ).toBe("No provider account");
  });
});
