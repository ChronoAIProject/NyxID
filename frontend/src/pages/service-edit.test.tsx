import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { ApiError } from "@/lib/api-client";
import { ServiceEditPage } from "./service-edit";

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
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (selector: (state: { user: { is_admin: boolean } }) => unknown) => selector({ user: { is_admin: true } }) }));
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
  it("omits unchanged skills on metadata edits", async () => {
    const user = userEvent.setup();
    render(<ServiceEditPage />);
    await user.clear(screen.getByLabelText("Service Name"));
    await user.type(screen.getByLabelText("Service Name"), "Renamed");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    const payload = mutate.mock.calls[0]![0].data;
    expect(payload.name).toBe("Renamed");
    expect(payload).not.toHaveProperty("recommended_skills");
    expect(payload).not.toHaveProperty("skills_revision");
    expect(payload).not.toHaveProperty("skills_request_id");
  });

  it("preserves observed revision through background refetch and retries identical payload with same ID", async () => {
    const user = userEvent.setup();
    mutate.mockRejectedValue(
      new ApiError(409, {
        error: "conflict",
        error_code: 1004,
        message: "Skills revision changed",
      }),
    );
    const view = render(<ServiceEditPage />);
    await user.clear(screen.getByLabelText("Recommended Skills"));
    await user.type(screen.getByLabelText("Recommended Skills"), "new");
    source.data = makeService({
      recommended_skills: ["someone-else"],
      skills_revision: 8,
    });
    view.rerender(<ServiceEditPage />);
    expect(screen.getByLabelText("Recommended Skills")).toHaveValue("new");
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    expect(mutate.mock.calls[0]![0].data.skills_revision).toBe(7);
    expect(await screen.findByText("Skills revision changed")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(2));
    expect(mutate.mock.calls[1]![0]).toEqual(mutate.mock.calls[0]![0]);
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
    await waitFor(() => expect(mutate).toHaveBeenCalledTimes(1));
    expect(mutate.mock.calls[0]![0].data).toMatchObject({
      recommended_skills: ["new"],
      clear_skill_refs: true,
      skills_revision: 2,
    });
  });
});
