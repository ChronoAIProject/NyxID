import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactElement } from "react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { OrgListItem } from "@/schemas/orgs";
import type { KeyInfo } from "@/types/keys";
import { DevicesOnboardPage } from "./devices-onboard";

const { mockOnboardMutateAsync, mockQrToDataUrl, mockToastSuccess, state } =
  vi.hoisted(() => ({
    mockOnboardMutateAsync: vi.fn(),
    mockQrToDataUrl: vi.fn(),
    mockToastSuccess: vi.fn(),
    state: {
      orgs: [] as OrgListItem[],
      orgsLoading: false,
      services: [] as KeyInfo[],
      servicesLoading: false,
    },
  }));

vi.mock("@/hooks/use-devices", () => ({
  useOnboardDevice: () => ({
    mutateAsync: mockOnboardMutateAsync,
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

vi.mock("qrcode", () => ({
  default: { toDataURL: mockQrToDataUrl },
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
  state.orgs = [];
  state.orgsLoading = false;
  state.services = [];
  state.servicesLoading = false;
  mockOnboardMutateAsync.mockResolvedValue({
    qr_payload: "nyxprov://full?ssid=Home&key=nyxid_ag_secret",
    node_id: "4df27e8f-8cb5-47b7-8d29-e6529f2c1c40",
    api_key_id: "7ef9c1a4-8df9-43af-9f92-98a6c9a7f45d",
    label: "Kitchen Camera",
  });
  mockQrToDataUrl.mockResolvedValue("data:image/png;base64,qr");
});

describe("DevicesOnboardPage", () => {
  it("submits onboard details and renders the generated QR image", async () => {
    const user = userEvent.setup();
    state.services = [
      makeKey({ id: "svc-personal", label: "Personal OpenAI" }),
    ];
    renderWithClient(<DevicesOnboardPage />);

    await fillOnboardForm(user);
    await user.click(screen.getByText("Personal OpenAI"));
    await user.click(screen.getByRole("button", { name: /generate qr/i }));

    await screen.findByText("Device onboarded");
    const qrImages = await screen.findAllByAltText("Device onboarding QR code");
    expect(
      qrImages.some(
        (img) => img.getAttribute("src") === "data:image/png;base64,qr",
      ),
    ).toBe(true);
    expect(mockOnboardMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        label: "Kitchen Camera",
        wifi_ssid: "HomeNetwork",
        wifi_password: "hunter22",
        default_services: ["svc-personal"],
      }),
    );
    expect(mockQrToDataUrl).toHaveBeenCalledWith(
      "nyxprov://full?ssid=Home&key=nyxid_ag_secret",
      expect.objectContaining({ width: 360 }),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith("Device onboarded");
  });

  it("filters grantable services by the selected org owner and prunes stale selections", async () => {
    const user = userEvent.setup();
    const orgId = "550e8400-e29b-41d4-a716-446655440000";
    state.orgs = [makeOrg({ id: orgId, display_name: "Acme Org" })];
    state.services = [
      makeKey({ id: "svc-personal", label: "Personal OpenAI" }),
      makeKey({
        id: "svc-org",
        label: "Org OpenAI",
        credential_source: {
          type: "org",
          org_id: orgId,
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
          org_id: orgId,
          org_name: "Acme Org",
          avatar_url: null,
          role: "viewer",
          allowed: false,
        },
      }),
    ];

    renderWithClient(<DevicesOnboardPage />);

    expect(screen.getByText("Personal OpenAI")).toBeInTheDocument();
    expect(screen.queryByText("Org OpenAI")).not.toBeInTheDocument();

    await user.click(screen.getByText("Personal OpenAI"));
    await user.click(screen.getByRole("combobox"));
    await user.click(await screen.findByRole("option", { name: "Acme Org" }));

    await waitFor(() =>
      expect(screen.getByText("Org OpenAI")).toBeInTheDocument(),
    );
    expect(screen.queryByText("Personal OpenAI")).not.toBeInTheDocument();
    expect(screen.queryByText("Viewer-only GitHub")).not.toBeInTheDocument();

    await fillOnboardForm(user);
    await user.click(screen.getByText("Org OpenAI"));
    await user.click(screen.getByRole("button", { name: /generate qr/i }));

    await screen.findByText("Device onboarded");
    expect(mockOnboardMutateAsync).toHaveBeenCalledWith(
      expect.objectContaining({
        org_id: orgId,
        default_services: ["svc-org"],
      }),
    );
  });

  it("renders a permission error when onboard is rejected", async () => {
    const user = userEvent.setup();
    mockOnboardMutateAsync.mockRejectedValue(
      new ApiError(403, {
        error: "forbidden",
        error_code: 1003,
        message: "forbidden",
      }),
    );
    renderWithClient(<DevicesOnboardPage />);

    await fillOnboardForm(user);
    await user.click(screen.getByRole("button", { name: /generate qr/i }));

    expect(
      await screen.findByText(
        "You do not have permission to onboard devices for that owner.",
      ),
    ).toBeInTheDocument();
  });

  it("renders a QR error when QR generation fails after onboard succeeds", async () => {
    const user = userEvent.setup();
    mockQrToDataUrl.mockRejectedValue(new Error("qr failed"));
    renderWithClient(<DevicesOnboardPage />);

    await fillOnboardForm(user);
    await user.click(screen.getByRole("button", { name: /generate qr/i }));

    expect(await screen.findByText("Device onboarded")).toBeInTheDocument();
    expect(
      await screen.findByText("QR code rendering failed."),
    ).toBeInTheDocument();
  });
});

async function fillOnboardForm(user: ReturnType<typeof userEvent.setup>) {
  await user.type(screen.getByLabelText("Label"), "Kitchen Camera");
  await user.type(screen.getByLabelText("WiFi SSID"), "HomeNetwork");
  await user.type(screen.getByLabelText("WiFi password"), "hunter22");
}

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
