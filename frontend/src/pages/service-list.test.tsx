import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServiceListPage } from "./service-list";

const { mockGet } = vi.hoisted(() => ({ mockGet: vi.fn() }));

vi.mock("@/lib/api-client", () => ({
  api: { get: mockGet, delete: vi.fn() },
}));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (selector: (state: { user: null }) => unknown) =>
    selector({ user: null }),
}));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
vi.mock("@/components/services/create-service-dialog", () => ({
  CreateServiceDialog: () => null,
}));

const providerIcon = "https://example.com/provider-icon.svg";
const ownIcon = "https://example.com/service-icon.svg";
const service = {
  id: "zoho-service",
  slug: "api-zoho-mail",
  name: "Zoho Mail",
  base_url: "https://mail.zoho.com",
  service_type: "http",
  auth_method: "none",
  offering_kind: "ai_service",
  icon_url: null,
  provider_config_id: "zoho-provider",
};
const providerResponse = {
  providers: [{ id: "zoho-provider", icon_url: providerIcon }],
};

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ServiceListPage />
    </QueryClientProvider>,
  );
}

describe("ServiceListPage icons", () => {
  beforeEach(() => {
    mockGet.mockReset();
  });

  it("inherits provider icons, keeps service overrides, and excludes Tools from inheritance", async () => {
    mockGet.mockImplementation(async (path: string) =>
      path === "/providers"
        ? providerResponse
        : {
            services: [
              service,
              { ...service, id: "override", name: "Override", icon_url: ownIcon },
              { ...service, id: "tool", name: "Tool", offering_kind: "tool" },
            ],
          },
    );
    const { container } = renderPage();
    await waitFor(() =>
      expect(container.querySelector(`img[src="${providerIcon}"]`)).toBeInTheDocument(),
    );
    expect(container.querySelectorAll(`img[src="${providerIcon}"]`)).toHaveLength(1);
    expect(container.querySelector(`img[src="${ownIcon}"]`)).toBeInTheDocument();
    expect(container.querySelector('[data-slug="api-zoho-mail"]')).toBeInTheDocument();
    expect(screen.getByText("Tool")).toBeInTheDocument();
  });

  it("shows services before provider metadata arrives, then fills in inherited icons", async () => {
    let resolveProviders!: (value: typeof providerResponse) => void;
    const pendingProviders = new Promise<typeof providerResponse>((resolve) => {
      resolveProviders = resolve;
    });
    mockGet.mockImplementation((path: string) =>
      path === "/providers" ? pendingProviders : Promise.resolve({ services: [service] }),
    );
    const { container } = renderPage();
    await screen.findByText("Zoho Mail");
    expect(container.querySelector('[data-slug="api-zoho-mail"]')).toBeInTheDocument();
    resolveProviders(providerResponse);
    await waitFor(() =>
      expect(container.querySelector(`img[src="${providerIcon}"]`)).toBeInTheDocument(),
    );
  });

  it("keeps services usable when optional provider metadata fails", async () => {
    mockGet.mockImplementation(async (path: string) => {
      if (path === "/providers") throw new Error("Providers unavailable");
      return { services: [service] };
    });
    const { container } = renderPage();
    await screen.findByText("Zoho Mail");
    expect(container.querySelector('[data-slug="api-zoho-mail"]')).toBeInTheDocument();
  });
});
