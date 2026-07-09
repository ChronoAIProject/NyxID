import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { NodeScopeCard } from "./node-scope-card";
import { ServiceScopeCard } from "./service-scope-card";

const { mockUseKeys, mockUseNodes } = vi.hoisted(() => ({
  mockUseKeys: vi.fn(),
  mockUseNodes: vi.fn(),
}));

vi.mock("@/hooks/use-keys", () => ({ useKeys: mockUseKeys }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: mockUseNodes }));
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), error: vi.fn() },
}));

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

function editButton(): HTMLElement {
  const button = screen
    .getAllByRole("button")
    .find((item) => item.querySelector(".lucide-pencil") !== null);
  if (!button) throw new Error("edit button not found");
  return button;
}

beforeEach(() => {
  mockUseKeys.mockReturnValue({
    data: [
      {
        id: "svc-personal",
        label: "Personal Service",
        slug: "personal-service",
        catalog_service_slug: "personal-service",
        auto_connected: false,
        is_active: true,
        credential_source: { type: "personal" },
      },
      {
        id: "svc-org-allowed",
        label: "Org Service",
        slug: "org-service",
        catalog_service_slug: "org-service",
        auto_connected: false,
        is_active: true,
        credential_source: {
          type: "org",
          org_id: "org-1",
          org_name: "Org One",
          role: "member",
          allowed: true,
        },
      },
      {
        id: "svc-org-viewer",
        label: "Viewer Service",
        slug: "viewer-service",
        catalog_service_slug: "viewer-service",
        auto_connected: false,
        is_active: true,
        credential_source: {
          type: "org",
          org_id: "org-2",
          org_name: "Org Two",
          role: "viewer",
          allowed: false,
        },
      },
    ],
  });
  mockUseNodes.mockReturnValue({
    data: [
      {
        id: "node-personal",
        name: "Personal Node",
        status: "Online",
        owner: { kind: "user", id: "user-1", display_name: "User One" },
      },
      {
        id: "node-org",
        name: "Org Node",
        status: "Online",
        owner: { kind: "org", id: "org-1", display_name: "Org One" },
      },
    ],
  });
});

describe("ServiceScopeCard", () => {
  it("lets personal API keys scope to proxyable org services", async () => {
    const user = userEvent.setup();
    render(
      <ServiceScopeCard
        keyId="key-1"
        allowAllServices={false}
        allowedServiceIds={[]}
        allowedServices={[]}
        apiKeySource={{ type: "personal" }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(editButton());

    expect(screen.getByText("Personal Service")).toBeInTheDocument();
    expect(screen.getByText("Org Service")).toBeInTheDocument();
    expect(screen.queryByText("Viewer Service")).not.toBeInTheDocument();
  });

  it("keeps org-owned API key service scopes owner-bound", async () => {
    const user = userEvent.setup();
    render(
      <ServiceScopeCard
        keyId="key-1"
        allowAllServices={false}
        allowedServiceIds={[]}
        allowedServices={[]}
        apiKeySource={{
          type: "org",
          org_id: "org-1",
          org_name: "Org One",
          role: "admin",
          allowed: true,
        }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(editButton());

    expect(screen.getByText("Org Service")).toBeInTheDocument();
    expect(screen.queryByText("Personal Service")).not.toBeInTheDocument();
    expect(screen.queryByText("Viewer Service")).not.toBeInTheDocument();
  });
});

describe("NodeScopeCard", () => {
  it("lets personal API keys scope to reachable personal and org nodes", async () => {
    const user = userEvent.setup();
    render(
      <NodeScopeCard
        keyId="key-1"
        allowAllNodes={false}
        allowedNodeIds={[]}
        allowedNodes={[]}
        apiKeySource={{ type: "personal" }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(editButton());

    expect(screen.getByText("Personal Node")).toBeInTheDocument();
    expect(screen.getByText("Org Node")).toBeInTheDocument();
  });

  it("keeps org-owned API key node scopes owner-bound", async () => {
    const user = userEvent.setup();
    render(
      <NodeScopeCard
        keyId="key-1"
        allowAllNodes={false}
        allowedNodeIds={[]}
        allowedNodes={[]}
        apiKeySource={{
          type: "org",
          org_id: "org-1",
          org_name: "Org One",
          role: "admin",
          allowed: true,
        }}
      />,
      { wrapper: createWrapper() },
    );

    await user.click(editButton());

    const scope = screen.getByText("Select allowed nodes:").parentElement;
    if (!scope) throw new Error("node scope list not found");
    expect(within(scope).getByText("Org Node")).toBeInTheDocument();
    expect(within(scope).queryByText("Personal Node")).not.toBeInTheDocument();
  });
});
