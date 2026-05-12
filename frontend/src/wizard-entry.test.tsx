import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const { mockUseOrgs } = vi.hoisted(() => ({
  mockUseOrgs: vi.fn(),
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: mockUseOrgs,
}));

vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    label,
  }: {
    readonly value: string | null;
    readonly onChange: (value: string | null) => void;
    readonly label?: string;
  }) => (
    <select
      aria-label={label ?? "Scope"}
      value={value ?? ""}
      onChange={() => {}}
    >
      <option value="">Personal</option>
      <option value="0a130a17-2624-4fbb-a69d-8ba51c99952a">ChronoAI</option>
    </select>
  ),
}));

import { ConfirmDispatcher, shouldShowDisconnectBanner } from "./wizard-entry";

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

beforeEach(() => {
  mockUseOrgs.mockReturnValue({
    data: [
      {
        id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
        display_name: "ChronoAI",
        your_role: "admin",
      },
    ],
  });
});

describe("shouldShowDisconnectBanner", () => {
  it("returns false when the CLI is still connected", () => {
    expect(shouldShowDisconnectBanner("claimed", false)).toBe(false);
  });

  it.each(["claimed", "secret", "acking"] as const)(
    "returns true for disconnected non-terminal phase %s",
    (phase) => {
      expect(shouldShowDisconnectBanner(phase, true)).toBe(true);
    },
  );

  it.each(["done", "cancelled", "wizard-lost"] as const)(
    "returns false for disconnected terminal phase %s",
    (phase) => {
      expect(shouldShowDisconnectBanner(phase, true)).toBe(false);
    },
  );
});

describe("ConfirmDispatcher ai-key prefill", () => {
  it("threads org_id into the ai-key owner picker", () => {
    render(
      <ConfirmDispatcher
        flow="ai-key"
        prefill={{
          custom: true,
          org_id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
          label: "ChronoAI PostHog",
          endpoint_url: "https://us.posthog.com",
          auth_method: "bearer",
        }}
        onSuccess={vi.fn()}
        onCancel={vi.fn()}
        onSlugPicked={vi.fn()}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("Owner")).toHaveValue(
      "0a130a17-2624-4fbb-a69d-8ba51c99952a",
    );
  });
});
