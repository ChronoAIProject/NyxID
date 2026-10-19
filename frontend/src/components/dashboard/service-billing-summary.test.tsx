import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ServiceInsightsState } from "@/hooks/use-service-insights";
import { configuredBilling } from "@/lib/service-insights-compat";
import type { KeyInfo } from "@/types/keys";
import { ServiceBillingSummary } from "./service-billing-summary";

const connection = {
  id: "personal",
  label: "Personal key",
  catalog_service_id: "catalog",
  catalog_service_slug: "service",
  credential_type: "api_key",
  credential_binding: "user",
  api_key_id: "key",
  auth_method: "bearer",
  auto_connected: false,
  is_active: true,
} as KeyInfo;
const platform = {
  ...connection,
  id: "platform",
  credential_binding: "platform" as const,
};
const priced = { slug: "service", billing: { platform_billable: true } };

function mount(
  connections: KeyInfo[],
  {
    free = false,
    status = "ready",
    restricted = false,
  }: {
    free?: boolean;
    status?: ServiceInsightsState["status"];
    restricted?: boolean;
  } = {},
) {
  const onOpen = vi.fn();
  render(
    <ServiceBillingSummary
      connections={connections}
      serviceName="Test service"
      onOpen={onOpen}
      insights={{
        status,
        refresh: vi.fn(),
        connections: new Map(
          connections.map((key) => [
            key.id,
            {
              service_id: key.id,
              billing: {
                ...configuredBilling(key, free ? { slug: "service" } : priced),
                ...(restricted ? { status: "restricted" as const } : {}),
              },
              usage: null,
            },
          ]),
        ),
      }}
    />,
  );
  const button = screen.getByRole("button", {
    name: "Show billing for Test service",
  });
  return { button, summary: within(button), onOpen };
}

describe("ServiceBillingSummary management label", () => {
  it.each([
    platform,
    { ...platform, is_active: false },
    {
      ...connection,
      credential_type: "oauth2",
      oauth_app_source: "platform" as const,
    },
    {
      ...connection,
      api_key_id: null,
      auth_method: "none",
      auto_connected: true,
    },
  ])(
    "identifies proven managed credentials or automatic connections: %j",
    (key) => {
      const { summary } = mount([key]);
      expect(summary.getByText("Platform managed")).toBeVisible();
    },
  );

  it("shows management for a free platform credential without implying a charge", () => {
    const { button, summary } = mount([platform], { free: true });
    expect(summary.getByText("—")).toBeVisible();
    expect(summary.getByText("Platform managed")).toBeVisible();
    expect(button).toHaveAccessibleDescription("Not billable by NyxID");
  });

  it("counts managed members in a mixed group and opens the platform billing panel", async () => {
    const { button, summary, onOpen } = mount([connection, platform]);
    expect(summary.getByText("1 NyxID managed · 1 BYOK")).toBeVisible();
    expect(summary.getByText("1 platform managed connection")).toBeVisible();
    await userEvent.click(button);
    expect(onOpen).toHaveBeenCalledWith(platform.id);
  });

  it.each([
    connection,
    { ...connection, api_key_id: null, auth_method: "none" },
    { ...connection, credential_type: "oauth2", oauth_app_source: null },
  ])(
    "does not infer management from billing or an unknown credential: %j",
    (key) => {
      const { summary } = mount([key]);
      expect(summary.queryByText(/platform managed/i)).not.toBeInTheDocument();
    },
  );

  it("hides management when billing provenance is restricted", () => {
    const { summary } = mount([platform], { restricted: true });
    expect(summary.queryByText(/platform managed/i)).not.toBeInTheDocument();
  });

  it.each(["loading", "unavailable", "restricted", "error"] as const)(
    "hides management during an insight %s state",
    (status) => {
      const { summary } = mount([platform], { status });
      expect(summary.queryByText(/platform managed/i)).not.toBeInTheDocument();
    },
  );
});
