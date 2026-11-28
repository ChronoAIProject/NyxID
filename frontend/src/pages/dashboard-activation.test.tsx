import type { ReactNode } from "react";
import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  FIRST_PROXY_CALL_SUCCEEDED_KEY,
  recordFirstProxyCallSucceeded,
} from "@/hooks/use-proxy-onboarding";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useApiKeys: () => ({ data: [{ is_active: true }], isLoading: false }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: [{ id: "svc-1" }], isLoading: false }),
}));
vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: [], isLoading: false }),
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

beforeEach(() => {
  localStorage.clear();
  useAuthStore.setState({
    user: { id: "user-a", display_name: "Test User" } as User,
    isAuthenticated: true,
  });
});

it("keeps the first-call step incomplete for the legacy unowned marker", () => {
  localStorage.setItem(FIRST_PROXY_CALL_SUCCEEDED_KEY, "1");
  render(<DashboardPage />);
  expect(screen.getAllByText("Make first proxy call").length).toBeGreaterThan(
    0,
  );
});

it("consumes success recorded on another route and retains it after remount", () => {
  recordFirstProxyCallSucceeded("user-a");
  const view = render(<DashboardPage />);
  expect(screen.queryByText("Make first proxy call")).not.toBeInTheDocument();
  view.unmount();
  render(<DashboardPage />);
  expect(screen.queryByText("Make first proxy call")).not.toBeInTheDocument();
});

it("updates from completion while mounted and resets for a different account", () => {
  render(<DashboardPage />);
  expect(screen.getAllByText("Make first proxy call").length).toBeGreaterThan(
    0,
  );
  act(() => recordFirstProxyCallSucceeded("user-a"));
  expect(screen.queryByText("Make first proxy call")).not.toBeInTheDocument();
  act(() => useAuthStore.setState({ user: { id: "user-b" } as User }));
  expect(screen.getAllByText("Make first proxy call").length).toBeGreaterThan(
    0,
  );
});
