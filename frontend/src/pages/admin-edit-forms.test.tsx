import {
  providerFormPayload,
  providerFormValues,
} from "./provider-edit.helpers";
import type { ReactNode } from "react";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { DownstreamService, ProviderConfig } from "@/types/api";
import { ProviderEditPage } from "./provider-edit";
import { ServiceEditPage } from "./service-edit";
import {
  serviceFormPatch,
  serviceFormPayload,
  serviceFormValues,
} from "./service-edit.helpers";
import { changedFields } from "@/lib/form-changes";
const mock = vi.hoisted(() => ({
  provider: { data: undefined as ProviderConfig | undefined, isLoading: false },
  service: {
    data: undefined as DownstreamService | undefined,
    isLoading: false,
  },
  updateProvider: vi.fn(),
  updateService: vi.fn(),
  navigate: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ providerId: "provider-1", serviceId: "service-1" }),
  useNavigate: () => mock.navigate,
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));
vi.mock("@/hooks/use-providers", () => ({
  useProvider: () => mock.provider,
  useUpdateProvider: () => ({
    mutateAsync: mock.updateProvider,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-services", () => ({
  useService: () => mock.service,
  useUpdateService: () => ({
    mutateAsync: mock.updateService,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-developer-apps", () => ({
  useDeveloperApps: () => ({
    data: {
      clients: [
        {
          id: "inactive-app",
          client_name: "Old app",
          client_type: "confidential",
          is_active: false,
        },
      ],
    },
  }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (s: unknown) => unknown) =>
    select({ user: { is_admin: true } }),
}));
vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({ data: { users: [] }, isFetching: false }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.provider = {
    isLoading: false,
    data: {
      id: "provider-1",
      slug: "test-provider",
      name: "Saved provider",
      provider_type: "oauth2",
      description: "Saved description",
      credential_mode: "both",
      is_active: true,
      authorization_url: "https://example.com/authorize",
      token_url: "https://example.com/token",
      revocation_url: "https://example.com/revoke",
      default_scopes: ["profile", "email"],
      supports_pkce: true,
      has_oauth_config: true,
      has_client_id: true,
      has_client_secret: true,
      token_endpoint_auth_method: "client_secret_basic",
      extra_auth_params: { audience: "test" },
      updated_at: "v1",
    } as unknown as ProviderConfig,
  };
  mock.service = {
    isLoading: false,
    data: {
      id: "service-1",
      name: "Saved service",
      slug: "service-1",
      description: "Saved description",
      base_url: "https://example.com",
      service_type: "http",
      visibility: "private",
      auth_type: "bearer",
      service_category: "internal",
      developer_app_ids: ["inactive-app", "unavailable-app"],
      billing: {
        platform_billable: true,
        resale_billable: true,
        lago_resale_metric_code: "retained",
        platform_pricing: { credits_per_unit: "2", sync_status: "synced" },
      },
      capabilities: { supports_streaming: true },
      updated_at: "v1",
    } as unknown as DownstreamService,
  };
  mock.updateProvider.mockResolvedValue(mock.provider.data);
  mock.updateService.mockResolvedValue(mock.service.data);
});
it("mounts populated provider controls only after loading", () => {
  const loaded = mock.provider.data;
  mock.provider = { isLoading: true, data: undefined };
  const view = render(<ProviderEditPage />);
  expect(
    screen.queryByRole("button", { name: "Save Changes" }),
  ).not.toBeInTheDocument();
  mock.provider = { isLoading: false, data: loaded };
  view.rerender(<ProviderEditPage />);
  expect(screen.getByLabelText("Authorization URL")).toHaveValue(
    "https://example.com/authorize",
  );
  expect(screen.getByLabelText("Token URL")).toHaveValue(
    "https://example.com/token",
  );
  expect(screen.getByLabelText("Revocation URL")).toHaveValue(
    "https://example.com/revoke",
  );
  expect(screen.getByLabelText(/Client ID/)).toHaveValue("");
  expect(screen.getByLabelText(/Client ID/)).toHaveAccessibleName(/Configured/);
  expect(screen.getByRole("button", { name: "Save Changes" })).toBeDisabled();
});
it("reviews and cancels, then submits only the provider name", async () => {
  const user = userEvent.setup();
  render(<ProviderEditPage />);
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "Renamed provider" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  const dialog = await screen.findByRole("dialog", { name: "Review changes" });
  expect(within(dialog).getByText("Saved provider")).toBeInTheDocument();
  expect(within(dialog).getByText("Renamed provider")).toBeInTheDocument();
  expect(mock.updateProvider).not.toHaveBeenCalled();
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(mock.updateProvider).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await user.click(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.updateProvider).toHaveBeenCalledExactlyOnceWith({
      name: "Renamed provider",
    }),
  );
});
it("preserves a draft and blocks confirmation after a background update", async () => {
  const user = userEvent.setup();
  const view = render(<ProviderEditPage />);
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "My draft" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  await screen.findByRole("dialog");
  mock.provider.data = {
    ...mock.provider.data!,
    name: "Other admin",
    updated_at: "v2",
  };
  view.rerender(<ProviderEditPage />);
  expect(screen.getByLabelText("Name")).toHaveValue("My draft");
  expect(
    screen.getByRole("button", { name: "Confirm changes" }),
  ).toBeDisabled();
  expect(mock.updateProvider).not.toHaveBeenCalled();
});
it("shows selected inactive and unavailable apps and patches only a service rename", async () => {
  const user = userEvent.setup();
  render(<ServiceEditPage />);
  expect(screen.getByLabelText("Old app (inactive)")).toBeChecked();
  expect(screen.getByText(/Selected app: unavailable-app/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Service Name"), {
    target: { value: "Renamed service" },
  });
  await user.click(screen.getByRole("button", { name: "Save Changes" }));
  expect(mock.updateService).not.toHaveBeenCalled();
  await user.click(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.updateService).toHaveBeenCalledExactlyOnceWith({
      serviceId: "service-1",
      data: { name: "Renamed service" },
    }),
  );
});
it("preserves node-key auth when editing an SSH host", () => {
  const service = {
    ...mock.service.data!,
    service_type: "ssh",
    ssh_config: {
      host: "old.example.com",
      port: 22,
      ssh_auth_mode: "node_key" as const,
      certificate_auth_enabled: false,
      certificate_ttl_minutes: 30,
      allowed_principals: ["ubuntu"],
      ca_public_key: null,
    },
  };
  const values = serviceFormValues(service);
  const before = serviceFormPayload(values, service);
  expect(
    changedFields(
      before,
      serviceFormPayload({ ...values, host: "new.example.com" }, service),
    ),
  ).toEqual({
    ssh_config: {
      host: "new.example.com",
      port: 22,
      ssh_auth_mode: "node_key",
      certificate_auth_enabled: false,
      certificate_ttl_minutes: 30,
      allowed_principals: ["ubuntu"],
    },
  });
});
it("preserves resale settings when changing the platform price", () => {
  const service = mock.service.data!;
  const values = serviceFormValues(service);
  const before = serviceFormPayload(values, service);
  expect(changedFields(before, serviceFormPayload(values, service))).toEqual(
    {},
  );
  expect(
    changedFields(
      before,
      serviceFormPayload({ ...values, platform_price: "3" }, service),
    ),
  ).toMatchObject({
    billing: {
      resale_billable: true,
      lago_resale_metric_code: "retained",
      platform_pricing: { credits_per_unit: "3" },
    },
  });
});

it("populates distinct device endpoints and encodes an explicit scope clear", () => {
  const provider = {
    ...mock.provider.data!,
    provider_type: "device_code" as const,
    device_code_url: "https://example.com/device",
    device_token_url: "https://example.com/device/poll",
  };
  const values = providerFormValues(provider);
  expect(values.token_url).toBe("https://example.com/token");
  expect(values.device_token_url).toBe("https://example.com/device/poll");
  expect(values.device_code_url).toBe("https://example.com/device");
  expect(
    changedFields(
      providerFormPayload(values),
      providerFormPayload({ ...values, default_scopes: "" }),
    ),
  ).toEqual({ default_scopes: [] });
});
it.each([
  ["cert", "proxy_only", "Cert", "Proxy Only"],
  ["proxy_only", "node_key", "Proxy Only", "Node Key"],
  ["node_key", "cert", "Node Key", "Cert"],
] as const)(
  "displays saved SSH mode %s and confirms a change to %s",
  async (mode, nextMode, label, nextLabel) => {
    const user = userEvent.setup();
    mock.service.data = {
      ...mock.service.data!,
      service_type: "ssh",
      ssh_config: {
        host: "host.example.com",
        port: 22,
        ssh_auth_mode: mode,
        certificate_auth_enabled: mode === "cert",
        certificate_ttl_minutes: 30,
        allowed_principals: ["ubuntu"],
        ca_public_key: null,
      },
    };
    render(<ServiceEditPage />);
    const selector = screen.getByRole("combobox", {
      name: "SSH Authentication Mode",
    });
    expect(selector).toHaveTextContent(label);
    expect(screen.getByLabelText("Allowed Principals")).toHaveValue("ubuntu");
    await user.click(selector);
    await user.click(screen.getByRole("option", { name: nextLabel }));
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    expect(mock.updateService).not.toHaveBeenCalled();
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() =>
      expect(mock.updateService).toHaveBeenCalledExactlyOnceWith({
        serviceId: "service-1",
        data: {
          ssh_config: {
            host: "host.example.com",
            port: 22,
            ssh_auth_mode: nextMode,
            certificate_auth_enabled: nextMode === "cert",
            certificate_ttl_minutes: 30,
            allowed_principals: ["ubuntu"],
          },
        },
      }),
    );
  },
);

it.each(["https://example.com/new-revoke", ""])(
  "preserves structured revocation settings or explicitly clears: %s",
  async (url) => {
    mock.provider.data = {
      ...mock.provider.data!,
      revocation: {
        url: "https://example.com/revoke",
        style: "rfc7009",
        auth: "basic",
        revokes_grant: true,
        request_encoding: "json",
      },
    } as ProviderConfig;
    const user = userEvent.setup();
    render(<ProviderEditPage />);
    fireEvent.change(screen.getByLabelText("Revocation URL"), {
      target: { value: url },
    });
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() =>
      expect(mock.updateProvider).toHaveBeenCalledWith({
        revocation: url ? { ...mock.provider.data!.revocation, url } : null,
      }),
    );
  },
);

it("ignores server-owned provider timestamps unless replacing a hidden credential", async () => {
  const view = render(<ProviderEditPage />);
  fireEvent.change(screen.getByLabelText("Name"), {
    target: { value: "Local" },
  });
  mock.provider.data = { ...mock.provider.data!, updated_at: "v2" };
  view.rerender(<ProviderEditPage />);
  expect(
    screen.queryByText(/Saved values changed while/),
  ).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText(/Client Secret/), {
    target: { value: "replacement" },
  });
  expect(screen.getByText(/Saved values changed while/)).toBeInTheDocument();
});

it("ignores service pricing synchronization and viewer metadata during an unrelated draft", () => {
  const view = render(<ServiceEditPage />);
  fireEvent.change(screen.getByLabelText("Service Name"), {
    target: { value: "Local" },
  });
  mock.service.data = {
    ...mock.service.data!,
    updated_at: "v2",
    your_binding_count: 2,
    billing: {
      ...mock.service.data!.billing!,
      platform_pricing: {
        ...mock.service.data!.billing!.platform_pricing!,
        sync_status: "failed",
        sync_error: "temporary",
      },
    },
  };
  view.rerender(<ServiceEditPage />);
  expect(
    screen.queryByText(/Saved values changed while/),
  ).not.toBeInTheDocument();
  expect(screen.getByLabelText("Service Name")).toHaveValue("Local");
});

it("preserves implicit platform access on rename and sends explicit inference/lane clears", () => {
  const service = {
    ...mock.service.data!,
    inference: {
      wire_protocol: "openai_responses" as const,
      model_list: true,
      realtime: false,
    },
    billing: {
      ...mock.service.data!.billing!,
      byok_pricing: {
        metric: "requests" as const,
        credits_per_unit: "3",
        sync_status: "synced" as const,
      },
      platform_key_pricing: {
        metric: "requests" as const,
        credits_per_unit: "4",
        sync_status: "synced" as const,
      },
      platform_charge_nyxid_credentials_only: true,
      platform_pricing_cleanup_metric_code: "cleanup-retained",
    },
  };
  const values = serviceFormValues(service);
  expect(values.platform_key).toBeUndefined();
  expect(values.credential).toBe("");
  expect(values.platform_charge_nyxid_credentials_only).toBe(true);
  expect(serviceFormPatch({ ...values, name: "Rename" }, service)).toEqual({
    name: "Rename",
  });
  expect(
    serviceFormPatch(
      { ...values, inference: null, byok_pricing: null },
      service,
    ),
  ).toEqual({ inference: null, billing: { byok_pricing: null } });
  const patch = serviceFormPatch(
    { ...values, platform_charge_nyxid_credentials_only: false },
    service,
  );
  expect(patch.billing).toMatchObject({
    platform_charge_nyxid_credentials_only: false,
    resale_billable: true,
    lago_resale_metric_code: "retained",
    platform_pricing_cleanup_metric_code: "cleanup-retained",
    platform_pricing: { credits_per_unit: "2" },
  });
  expect(patch.billing).not.toHaveProperty("byok_pricing");
  expect(patch.billing).not.toHaveProperty("platform_key_pricing");
});

it("shows platform credential replacement only as a redacted review row", async () => {
  render(<ServiceEditPage />);
  fireEvent.change(screen.getByLabelText("Replace platform credential"), {
    target: { value: "replacement-secret-material" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
  const dialog = await screen.findByRole("dialog", { name: "Review changes" });
  expect(dialog).not.toHaveTextContent("replacement-secret-material");
  expect(mock.updateService).not.toHaveBeenCalled();
  fireEvent.click(
    within(dialog).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.updateService).toHaveBeenCalledWith({
      serviceId: "service-1",
      data: { credential: "replacement-secret-material" },
    }),
  );
});
