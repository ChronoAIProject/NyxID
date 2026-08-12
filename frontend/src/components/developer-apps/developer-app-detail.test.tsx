import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OAuthClient } from "@/types/api";
import type { CatalogEntry } from "@/types/keys";

const mocks = vi.hoisted(() => ({
  deleteMutateAsync: vi.fn(),
  navigate: vi.fn(),
  rotateMutateAsync: vi.fn(),
  toastError: vi.fn(),
  toastSuccess: vi.fn(),
  updateMutateAsync: vi.fn(),
  useCatalog: vi.fn(),
  useDeveloperApp: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-developer-apps", () => ({
  useDeleteDeveloperApp: () => ({
    mutateAsync: mocks.deleteMutateAsync,
    isPending: false,
  }),
  useDeveloperApp: mocks.useDeveloperApp,
  useRotateDeveloperAppSecret: () => ({
    mutateAsync: mocks.rotateMutateAsync,
    isPending: false,
  }),
  useUpdateDeveloperApp: () => ({
    mutateAsync: mocks.updateMutateAsync,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: mocks.useCatalog,
}));

vi.mock("@/components/shared/client-secret-dialog", () => ({
  ClientSecretDialog: ({ open }: { readonly open: boolean }) =>
    open ? <div data-testid="client-secret-dialog" /> : null,
}));

vi.mock("./connection-webhook-section", () => ({
  ConnectionWebhookSection: ({
    clientId,
  }: {
    readonly clientId: string;
  }) => <div data-testid="connection-webhook-section">{clientId}</div>,
}));

vi.mock("sonner", () => ({
  toast: {
    error: mocks.toastError,
    success: mocks.toastSuccess,
  },
}));

import { DeveloperAppDetail } from "./developer-app-detail";

const oauthClient: OAuthClient = {
  id: "client-1",
  client_name: "Acme OAuth",
  client_type: "confidential",
  redirect_uris: ["https://app.example.com/callback"],
  allowed_scopes: "openid profile email",
  delegation_scopes: "",
  broker_capability_enabled: false,
  revocation_webhook_url: null,
  connection_webhook_url: null,
  connection_webhook_enabled: false,
  is_active: true,
  default_service_catalog_slugs: [],
  client_secret: null,
  created_at: "2026-04-20T00:00:00Z",
};

const catalogEntries = [
  {
    slug: "openai",
    name: "OpenAI",
  },
  {
    slug: "chrono-llm-public",
    name: "Chrono LLM Public",
  },
] as unknown as readonly CatalogEntry[];

describe("DeveloperAppDetail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.useDeveloperApp.mockReturnValue({
      data: oauthClient,
      isLoading: false,
    });
    mocks.useCatalog.mockReturnValue({
      data: catalogEntries,
      isLoading: false,
    });
    mocks.updateMutateAsync.mockResolvedValue(oauthClient);
  });

  it("uses the include-all catalog for default service declarations", async () => {
    const user = userEvent.setup();

    render(
      <DeveloperAppDetail
        clientId="client-1"
        backTo={{ to: "/developer-apps", label: "Developer Apps" }}
      />,
    );

    expect(mocks.useCatalog).toHaveBeenCalledWith({ includeAll: true });

    await user.click(screen.getByRole("button", { name: "Edit" }));

    expect(screen.getByText("Chrono LLM Public")).toBeInTheDocument();
    expect(screen.getByText("chrono-llm-public")).toBeInTheDocument();

    await user.click(
      screen.getByRole("checkbox", { name: /Chrono LLM Public/ }),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(mocks.updateMutateAsync).toHaveBeenCalledWith({
        clientId: "client-1",
        data: {
          name: "Acme OAuth",
          redirect_uris: ["https://app.example.com/callback"],
          allowed_scopes: ["openid", "profile", "email"],
          default_service_catalog_slugs: ["chrono-llm-public"],
        },
      });
    });
  });
});
