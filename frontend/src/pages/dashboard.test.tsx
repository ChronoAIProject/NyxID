import type { ReactNode } from "react";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { NodeInfo } from "@/types/nodes";

const state = vi.hoisted(() => ({
  nodes: undefined as readonly NodeInfo[] | undefined,
  nodesLoading: false,
  servicesLoading: false,
}));

vi.mock("@/i18n/use-ui-translation", () => ({
  translateUi: (key: string) => key,
  useUiTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (value: { user: { display_name: string; email: string; email_verified: boolean; mfa_enabled: boolean } }) => unknown) =>
    selector({
      user: {
        display_name: "Test User",
        email: "test@example.com",
        email_verified: true,
        mfa_enabled: true,
      },
    }),
}));

vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: () => ({ data: [], isLoading: false }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: [], isLoading: state.servicesLoading }),
}));
vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: state.nodes, isLoading: state.nodesLoading }),
}));
vi.mock("@/hooks/use-proxy-onboarding", () => ({
  useProxyOnboarding: () => ({
    firstProxyCallSucceeded: false,
    verifyKeyLoading: false,
  }),
}));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: undefined }),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useRightPanel: () => ({ setRightPanel: vi.fn() }),
}));
vi.mock("@/components/dashboard/agent-plugins-section", () => ({
  AgentPluginsSection: () => null,
}));

import { DashboardPage } from "./dashboard";

function makeNode(
  id: string,
  status: string,
  isConnected: boolean,
): NodeInfo {
  return {
    id,
    name: id,
    owner: { kind: "user", id: "user-1", display_name: "Test User" },
    status,
    is_connected: isConnected,
    last_heartbeat_at: null,
    connected_at: null,
    metadata: null,
    metrics: null,
    binding_count: 0,
    created_at: "2026-01-01T00:00:00Z",
  };
}

afterEach(() => {
  cleanup();
  state.nodes = undefined;
  state.nodesLoading = false;
  state.servicesLoading = false;
  localStorage.clear();
  vi.clearAllMocks();
});

it("counts connected nodes using the same live status rule as NodesPage", () => {
  state.nodes = [
    makeNode("online-connected", "online", true),
    makeNode("online-stale", "online", false),
    makeNode("draining-connected", "draining", true),
    makeNode("offline", "offline", false),
  ];

  render(<DashboardPage />);

  expect(screen.getByText("2/4 online")).toBeInTheDocument();
});

it("renders the empty node state when the API returns no nodes", () => {
  state.nodes = [];

  render(<DashboardPage />);

  expect(screen.getByText("No nodes")).toBeInTheDocument();
  expect(screen.queryByText(/\/0 online/)).not.toBeInTheDocument();
});

it("keeps the dashboard loading while services are unresolved", () => {
  state.nodes = [makeNode("online-connected", "online", true)];
  state.servicesLoading = true;

  const { container } = render(<DashboardPage />);

  expect(screen.queryByText("Welcome back, {{name}}"),).not.toBeInTheDocument();
  expect(screen.queryByText("No nodes")).not.toBeInTheDocument();
  expect(container.querySelector(".animate-pulse")).toBeInTheDocument();
});
