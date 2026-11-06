import { createRoot, type Root } from "react-dom/client";
import { useEffect, useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";
import { useApplyTheme } from "../../src/hooks/use-theme";
import { useThemeStore } from "../../src/stores/theme-store";
import { SignInStep } from "../../src/features/nyxbot-onboarding/sign-in-step";
import { nyxbotI18n } from "../../src/features/nyxbot-onboarding/i18n";
import { AvailableBenefits } from "../../src/components/billing/available-benefits";
import { useAuthStore } from "../../src/stores/auth-store";
import { BillingWalletActions } from "../../src/components/billing/billing-wallet-card";
import { BillingQuantityChart } from "../../src/components/billing/billing-quantity-chart";
import { AnalyticsCanvas } from "../../src/components/billing-analytics/analytics-canvas";
import { sampleAnalytics } from "../../src/components/billing-analytics/sample-data";
import { GroupedServiceCards } from "../../src/components/dashboard/grouped-service-cards";
import { TooltipProvider } from "../../src/components/ui/tooltip";
import { newView } from "../../src/lib/usage-analytics";
import {
  billingUsageRowSchema,
  billingWalletResponseSchema,
} from "../../src/schemas/billing";
import type { KeyInfo } from "../../src/types/keys";
import "../../src/app.css";
import "../../src/components/billing/billing-page.css";
import "../../src/features/nyxbot-onboarding/onboarding.css";

const wallet = billingWalletResponseSchema.parse({
  owner_id: "local-display-fixture",
  plan_kind: "prepaid",
  collection_state: "good",
  balance: "-8.180326",
  balance_credits: -8,
  reserved_credits: 0,
  pending_lago_debits: 0,
  available: "-8.180326",
  available_credits: -8,
  available_with_overdraft_credits: 92,
  has_payment_instrument: false,
  overdraft_cap_credits: 100,
  suspended: false,
  lago_customer_id: "local-display-fixture",
  balance_synced_at: "2026-11-04T00:00:00Z",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
  created: false,
});
const rows = ["gpt-large", "gpt-medium", "gpt-small"].map((model, i) =>
  billingUsageRowSchema.parse({
    metric: "input_tokens",
    quantity: (i + 1) * 12345,
    model,
    layer: "platform",
    service_slug: "review",
    lago_metric_code: "review",
    requests: 1,
    bytes: 0,
    events: 1,
    lago_acked: true,
    estimated_credits_micros: 0,
  }),
);
const connection: KeyInfo = {
  id: "review-service",
  label: "Aevatar",
  slug: "aevatar",
  endpoint_id: "review-endpoint",
  credential_type: "api_key",
  auth_method: "header",
  auth_key_name: "Authorization",
  status: "active",
  catalog_service_id: "aevatar",
  catalog_service_slug: "aevatar",
  catalog_service_name: "Aevatar",
  node_id: null,
  node_priority: 0,
  is_active: true,
  ws_frame_injections: [],
  auto_connected: false,
  expires_at: null,
  last_used_at: null,
  error_message: null,
  created_at: "2026-11-04T00:00:00Z",
  service_type: "http",
  ssh_host: null,
  ssh_port: null,
  ssh_ca_public_key: null,
  ssh_allowed_principals: null,
  ssh_certificate_ttl_minutes: null,
};
const client = new QueryClient({
  defaultOptions: { queries: { retry: false, staleTime: Infinity } },
});
const fixtureUserId = useAuthStore.getState().user?.id;
client.setQueryData(["billing", "wallet", fixtureUserId], wallet);
client.setQueryData(
  ["billing", "credits", "benefits", fixtureUserId, undefined, 1],
  {
    summary: {
      remaining: "0",
      unreserved: "0",
      held: "0",
      pending_activation: "0",
      pending_reclaim: "0",
      expired_awaiting_cleanup: "0",
      restricted: "0",
      generated_at: "2026-11-04T00:00:00Z",
    },
    grants: [],
    allowances: [],
    total_grants: 0,
    page: 1,
    per_page: 50,
  },
);
client.setQueryData(["public-config"], {
  social_providers: ["google", "github", "apple"],
  email_auth_enabled: true,
});

declare global {
  interface Window {
    customDisplayFixture: {
      set: (
        scale: number,
        theme: "light" | "dark",
        view: "billing" | "signin",
      ) => void;
    };
  }
}

export function CustomDisplayFixture() {
  useApplyTheme(
    location.pathname.startsWith("/assistant") ? "assistant" : undefined,
  );
  const [surface, setSurface] = useState<"billing" | "signin">("billing");
  const [view, setView] = useState(() => {
    const value = newView("operations");
    return { ...value, panels: value.panels.slice(0, 1) };
  });
  useEffect(() => {
    window.customDisplayFixture = {
      set: (textScale, mode, next) => {
        useThemeStore.setState({ textScale, mode });
        setSurface(next);
      },
    };
  }, []);
  return (
    <TooltipProvider>
      {surface === "signin" ? (
        <I18nextProvider i18n={nyxbotI18n}>
          <SignInStep
            returnTo="/assistant"
            accountName="Display reader"
            onContinue={() => {}}
          />
        </I18nextProvider>
      ) : (
        <main className="mx-auto max-w-[900px] space-y-6 p-4 billing-page">
          <section aria-label="Wallet fixture">
            <AvailableBenefits
              actions={
                <BillingWalletActions
                  wallet={wallet}
                  loading={false}
                  unavailable={false}
                  error={null}
                  onRetry={() => {}}
                  onProvision={() => {}}
                  provisioning={false}
                  billingReady
                  onTopUp={async () => {}}
                  topUpPending={false}
                />
              }
            />
          </section>
          <section aria-label="Quantity fixture">
            <BillingQuantityChart rows={rows} catalog={[]} />
          </section>
          <section aria-label="Analytics fixture">
            <AnalyticsCanvas
              view={view}
              onChange={setView}
              sample={sampleAnalytics}
            />
          </section>
          <section aria-label="Service fixture">
            <GroupedServiceCards keys={[connection]} />
          </section>
        </main>
      )}
    </TooltipProvider>
  );
}
const router = createRouter({
  routeTree: createRootRoute({ component: CustomDisplayFixture }),
  history: createMemoryHistory({ initialEntries: ["/"] }),
});
const container = document.getElementById("root")! as HTMLElement & {
  fixtureRoot?: Root;
};
const root = (container.fixtureRoot ??= createRoot(container));
root.render(
  <QueryClientProvider client={client}>
    <RouterProvider router={router} />
  </QueryClientProvider>,
);
