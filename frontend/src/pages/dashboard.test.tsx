import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { DashboardPage } from "./dashboard";

vi.mock("@tanstack/react-router", () => ({ Link: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
vi.mock("@/hooks/use-api-keys", () => ({ useApiKeys: () => ({ data: [], isLoading: false }) }));
vi.mock("@/hooks/use-keys", () => ({ useKeys: () => ({ data: [], isLoading: false }) }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [
  { status: "online", is_connected: true },
  { status: "offline", is_connected: false },
  { status: "draining", is_connected: false },
  { status: "online", is_connected: false },
], isLoading: false }) }));
vi.mock("@/hooks/use-public-config", () => ({ usePublicConfig: () => ({ data: undefined }) }));
vi.mock("@/hooks/use-proxy-onboarding", () => ({ useProxyOnboarding: () => ({ firstProxyCallSucceeded: true, verifyKeyLoading: false }) }));
vi.mock("@/components/layout/dashboard-layout", () => ({ useRightPanel: () => ({ setRightPanel: vi.fn() }) }));
vi.mock("@/components/dashboard/agent-plugins-section", () => ({ AgentPluginsSection: () => null }));
afterEach(cleanup);

it("counts connected nodes using lowercase API statuses, excluding offline and draining nodes", () => {
  render(<DashboardPage />);
  expect(screen.getByText("1/4 online")).toBeInTheDocument();
});
