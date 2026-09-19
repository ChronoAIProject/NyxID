import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { ApiError } from "@/lib/api-client";
import { ServiceEditPage } from "./service-edit";
import { serviceFormPatch, serviceFormValues } from "./service-edit.helpers";

const { source, mutate } = vi.hoisted(() => ({
  source: { data: undefined as DownstreamService | undefined },
  mutate: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  useParams: () => ({ serviceId: source.data?.id }),
}));
vi.mock("@/hooks/use-services", () => ({
  useService: () => ({
    ...source,
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  }),
  useUpdateService: () => ({ mutateAsync: mutate, isPending: false }),
}));
vi.mock("@/hooks/use-developer-apps", () => ({
  useDeveloperApps: () => ({ data: { clients: [] } }),
}));
vi.mock("@/components/shared/page-header", () => ({
  PageHeader: ({ title }: { title: string }) => <h1>{title}</h1>,
}));
vi.mock("@/components/dashboard/identity-propagation-config", () => ({
  IdentityPropagationConfig: () => null,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (
    selector: (state: { user: { is_admin: boolean } }) => unknown,
  ) => selector({ user: { is_admin: true } }),
}));
vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({ data: { users: [] }, isFetching: false }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

function makeService(
  overrides: Partial<DownstreamService> = {},
): DownstreamService {
  return {
    id: "svc-1",
    name: "Public API",
    slug: "public-api",
    description: "Public API",
    base_url: "https://api.example.test",
    service_type: "http",
    visibility: "public",
    auth_method: "none",
    auth_type: "none",
    auth_key_name: "",
    is_active: true,
    oauth_client_id: null,
    api_spec_url: null,
    service_category: "connection",
    requires_user_credential: true,
    created_by: "user-1",
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
    identity_propagation_mode: "none",
    forward_access_token: false,
    inject_delegation_token: false,
    anonymous_endpoints: [],
    default_request_headers: null,
    ws_frame_injections: null,
    node_id: null,
    your_user_service_id: null,
    your_binding_count: 0,
    ...overrides,
    effective_platform_metric:
      overrides.effective_platform_metric ?? "requests",
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  source.data = makeService({
    recommended_skills: ["old"],
    skills_revision: 7,
  });
  mutate.mockResolvedValue({});
});

describe("service editor curation concurrency", () => {
  it("preserves component prices on unrelated edits and sends only editable price fields", () => {
    const service = makeService({
      billing: {
        platform_billable: false,
        resale_billable: false,
        resale_metric: "tokens",
        byok_pricing: {
          metric: "input_tokens",
          credits_per_unit: "0.000000000001",
          sync_status: "synced",
          components: [
            {
              metric: "cache_read_tokens",
              credits_per_unit: "0.000000250001",
              sync_status: "synced",
            },
          ],
        },
      },
    });
    const values = serviceFormValues(service);
    expect(serviceFormPatch(values, service)).toEqual({});
    expect(
      serviceFormPatch({ ...values, name: "Renamed" }, service),
    ).not.toHaveProperty("billing");
    const changed = {
      ...values,
      byok_pricing: {
        ...values.byok_pricing!,
        credits_per_unit: "0.000000000002",
      },
    };
    const patch = serviceFormPatch(changed, service);
    expect(patch.billing?.byok_pricing).toEqual({
      metric: "input_tokens",
      credits_per_unit: "0.000000000002",
      components: [
        { metric: "cache_read_tokens", credits_per_unit: "0.000000250001" },
      ],
    });
  });

  it("omits unchanged skills on metadata edits", async () => {
    const user = userEvent.setup();
    render(<ServiceEditPage />);
    await user.clear(screen.getByLabelText("Service Name"));
    await user.type(screen.getByLabelText("Service Name"), "Renamed");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    const payload = mutate.mock.calls[0]![0].data;
    expect(payload.name).toBe("Renamed");
    expect(payload).not.toHaveProperty("recommended_skills");
    expect(payload).not.toHaveProperty("skills_revision");
    expect(payload).not.toHaveProperty("skills_request_id");
  });

  it("retries the reviewed skill payload with the same observed revision and request ID", async () => {
    const user = userEvent.setup();
    mutate.mockRejectedValue(
      new ApiError(409, {
        error: "conflict",
        error_code: 1004,
        message: "Skills revision changed",
      }),
    );
    render(<ServiceEditPage />);
    await user.clear(screen.getByLabelText("Recommended Skills"));
    await user.type(screen.getByLabelText("Recommended Skills"), "new");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    expect(mutate.mock.calls[0]![0].data.skills_revision).toBe(7);
    expect(
      await screen.findAllByText("Skills revision changed"),
    ).not.toHaveLength(0);
    await user.click(screen.getByRole("button", { name: "Confirm changes" }));
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(2));
    expect(mutate.mock.calls[1]![0]).toEqual(mutate.mock.calls[0]![0]);
  });

  it("preserves a draft and blocks a pending review after a ref-only background change", async () => {
    const user = userEvent.setup();
    const view = render(<ServiceEditPage />);
    await user.clear(screen.getByLabelText("Recommended Skills"));
    await user.type(screen.getByLabelText("Recommended Skills"), "new");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await screen.findByRole("dialog");
    source.data = {
      ...source.data!,
      skills_revision: 8,
      recommended_skill_refs: [],
    };
    view.rerender(<ServiceEditPage />);
    expect(screen.getByLabelText("Recommended Skills")).toHaveValue("new");
    expect(
      screen.getByRole("button", { name: "Confirm changes" }),
    ).toBeDisabled();
    expect(mutate).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(
      screen.getByRole("button", {
        name: "Load latest values (discard edits)",
      }),
    );
    expect(screen.getByLabelText("Recommended Skills")).toHaveValue("old");
    expect(
      screen.getByRole("checkbox", {
        name: "Clear pinned references and use advisory names",
      }),
    ).not.toBeChecked();
  });

  it("offers explicit ref clearing even for an empty pinned list", async () => {
    const user = userEvent.setup();
    source.data = makeService({
      recommended_skills: [],
      recommended_skill_refs: [],
      skills_revision: 2,
    });
    render(<ServiceEditPage />);
    await user.type(screen.getByLabelText("Recommended Skills"), "new");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    expect(mutate).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("checkbox", {
        name: "Clear pinned references and use advisory names",
      }),
    );
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await user.click(
      await screen.findByRole("button", { name: "Confirm changes" }),
    );
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    expect(mutate.mock.calls[0]![0].data).toMatchObject({
      recommended_skills: ["new"],
      clear_skill_refs: true,
      skills_revision: 2,
    });
  });
});
