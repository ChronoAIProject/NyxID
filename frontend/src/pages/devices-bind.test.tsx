import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { OrgListItem } from "@/schemas/orgs";
import type { KeyInfo } from "@/types/keys";
import { DevicesBindPage } from "./devices-bind";

const { mockApproveMutateAsync, mockToastSuccess, state } = vi.hoisted(() => ({
  mockApproveMutateAsync: vi.fn(),
  mockToastSuccess: vi.fn(),
  state: {
    search: {} as { user_code?: string },
    orgs: [] as OrgListItem[],
    orgsLoading: false,
    services: [] as KeyInfo[],
    servicesLoading: false,
  },
}));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => state.search,
}));

vi.mock("@/hooks/use-devices", () => ({
  useApproveDevice: () => ({
    mutateAsync: mockApproveMutateAsync,
    isPending: false,
  }),
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: state.services,
    isLoading: state.servicesLoading,
  }),
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({
    data: state.orgs,
    isLoading: state.orgsLoading,
  }),
}));

vi.mock("sonner", () => ({
  toast: { success: mockToastSuccess },
}));

beforeAll(() => {
  if (!Element.prototype.hasPointerCapture) {
    Element.prototype.hasPointerCapture = () => false;
  }
  if (!Element.prototype.setPointerCapture) {
    Element.prototype.setPointerCapture = () => {};
  }
  if (!Element.prototype.releasePointerCapture) {
    Element.prototype.releasePointerCapture = () => {};
  }
  if (!Element.prototype.scrollIntoView) {
    Element.prototype.scrollIntoView = () => {};
  }
});

beforeEach(() => {
  vi.clearAllMocks();
  state.search = {};
  state.orgs = [];
  state.orgsLoading = false;
  state.services = [];
  state.servicesLoading = false;
  mockApproveMutateAsync.mockResolvedValue({
    device_label: "Hall camera",
    hw_id: "esp32-aabbcc",
    api_key_id: "7ef9c1a4-8df9-43af-9f92-98a6c9a7f45d",
    node_id: "4df27e8f-8cb5-47b7-8d29-e6529f2c1c40",
    owner_user_id: "user-1",
    org_id: null,
  });
});

describe("DevicesBindPage", () => {
  it("submits approval and renders the approved device summary", async () => {
    const user = userEvent.setup();
    state.search = { user_code: "abcd efgh jklm" };
    state.services = [
      makeKey({ id: "svc-personal", label: "Personal OpenAI" }),
    ];
    renderWithClient(<DevicesBindPage />);

    await user.type(screen.getByLabelText("Label"), "Hall camera");
    await user.click(screen.getByText("Personal OpenAI"));
    await user.click(screen.getByRole("button", { name: /approve device/i }));

    await screen.findByText("Device approved");
    expect(screen.getByText("Hall camera")).toBeInTheDocument();
    expect(mockApproveMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        user_code: "ABCD-EFGH-JKLM",
        label: "Hall camera",
        default_services: ["svc-personal"],
      }),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith("Device approved");
  });

  it("renders a device-code error when approval fails", async () => {
    const user = userEvent.setup();
    mockApproveMutateAsync.mockRejectedValue(
      new ApiError(400, {
        error: "device_user_code_invalid",
        error_code: 9503,
        message: "invalid user code",
      }),
    );
    renderWithClient(<DevicesBindPage />);

    await user.type(screen.getByLabelText("User code"), "ABCD-EFGH-JKLM");
    await user.click(screen.getByRole("button", { name: /approve device/i }));

    expect(
      await screen.findByText("That device code is not valid."),
    ).toBeInTheDocument();
  });

  it("filters grantable services by the selected owner", async () => {
    const user = userEvent.setup();
    state.orgs = [makeOrg({ id: "org-1", display_name: "Acme Org" })];
    state.services = [
      makeKey({ id: "svc-personal", label: "Personal OpenAI" }),
      makeKey({
        id: "svc-org",
        label: "Org OpenAI",
        credential_source: {
          type: "org",
          org_id: "org-1",
          org_name: "Acme Org",
          avatar_url: null,
          role: "admin",
          allowed: true,
        },
      }),
      makeKey({
        id: "svc-viewer",
        label: "Viewer-only GitHub",
        credential_source: {
          type: "org",
          org_id: "org-1",
          org_name: "Acme Org",
          avatar_url: null,
          role: "viewer",
          allowed: false,
        },
      }),
    ];

    renderWithClient(<DevicesBindPage />);

    expect(screen.getByText("Personal OpenAI")).toBeInTheDocument();
    expect(screen.queryByText("Org OpenAI")).not.toBeInTheDocument();

    await user.click(screen.getByRole("combobox"));
    await user.click(await screen.findByRole("option", { name: "Acme Org" }));

    await waitFor(() =>
      expect(screen.getByText("Org OpenAI")).toBeInTheDocument(),
    );
    expect(screen.queryByText("Personal OpenAI")).not.toBeInTheDocument();
    expect(screen.queryByText("Viewer-only GitHub")).not.toBeInTheDocument();
  });
});

function renderWithClient(children: ReactElement) {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>,
  );
}

function makeOrg(overrides: Partial<OrgListItem> = {}): OrgListItem {
  return {
    id: "org-1",
    slug: "acme",
    display_name: "Acme Org",
    avatar_url: null,
    contact_email: null,
    your_role: "admin",
    created_at: "2026-06-01T00:00:00Z",
    ...overrides,
  };
}

function makeKey(overrides: Partial<KeyInfo> = {}): KeyInfo {
  return {
    id: "svc-1",
    label: "Personal OpenAI",
    slug: "openai",
    endpoint_url: "https://api.openai.com",
    endpoint_id: "ep-1",
    credential_type: "bearer",
    auth_method: "bearer",
    auth_key_name: "Authorization",
    status: "active",
    catalog_service_id: "cat-1",
    catalog_service_slug: "openai",
    catalog_service_name: "OpenAI",
    node_id: null,
    node_priority: 0,
    is_active: true,
    ws_frame_injections: [],
    auto_connected: false,
    expires_at: null,
    last_used_at: null,
    error_message: null,
    created_at: "2026-06-01T00:00:00Z",
    service_type: "http",
    ssh_host: null,
    ssh_port: null,
    ssh_ca_public_key: null,
    ssh_allowed_principals: null,
    ssh_certificate_ttl_minutes: null,
    ...overrides,
  };
}
