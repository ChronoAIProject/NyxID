import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, afterEach, describe, it, expect, vi } from "vitest";
import type { ReactNode } from "react";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { PlatformModelsPage } from "./platform-models";
import {
  AdminPlatformModelsPage,
  AdminPlatformModelDetailPage,
} from "./admin-platform-models";
import {
  publicModelSchema,
  modelFormSchema,
  modelDefaults,
  type AdminModel,
} from "@/schemas/platform-models";
import { getVisibleMainNav } from "@/components/dashboard/sidebar";

const mock = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn(), put: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: mock,
}));
vi.mock("@tanstack/react-router", async (original) => ({
  ...(await original<typeof import("@tanstack/react-router")>()),
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
  useNavigate: () => vi.fn(),
  useParams: () => ({ modelId: "00000000-0000-4000-8000-000000000001" }),
}));
const id = "00000000-0000-4000-8000-000000000001";
const other = "00000000-0000-4000-8000-000000000002";
const capabilities = {
  protocols: ["openai_chat"],
  input_modalities: ["text"],
  output_modalities: ["text"],
};
const model: AdminModel = {
  id,
  public_id: "nyxid/gpt-test",
  display_name: "Test model",
  description: "Public description",
  family: "openai",
  capabilities,
  lifecycle: "draft",
  enabled: false,
  revision: 7,
  authorization_revision: 2,
  billing_service_id: other,
  entitlements: { all_people: false, rules: [] },
  routing_revision_id: null,
  routes: [],
  execution_ready: false,
  blocking_reason: "execution_fleet_not_ready",
  required_readiness: [
    "inference:platform-model-registry-v1",
    "inference:platform-models-v1",
    "billing:model-prices-v1",
  ],
  created_at: "2026-11-01T00:00:00Z",
  updated_at: "2026-11-01T00:00:00Z",
};
const component = {
  key: "input",
  metric: "input_tokens",
  selector: { kind: "input" },
  credits_per_unit: "0.000000000001",
  explicit_free: false,
  max_quantity: 100,
  lago_metric_code: "private_metric",
};
const publicModel = {
  id: model.public_id,
  display_name: model.display_name,
  description: model.description,
  capabilities,
  availability: "unavailable",
  retail_prices: [{ available: true, variant: {}, components: [component] }],
};
function account(enabled = true, role = "admin") {
  useAuthStore.setState({
    user: {
      id: "person",
      role,
      is_admin: role === "admin",
      capabilities: {
        enabled_features: enabled ? ["inference:platform-models"] : [],
      },
    } as never,
  });
}
function show(node: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>{node}</QueryClientProvider>,
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  account();
  mock.get.mockImplementation(async (path: string) => {
    if (path.startsWith("/llm/models"))
      return { models: [publicModel], next_after: null };
    if (path === `/admin/platform-models/${id}`) return model;
    if (path.startsWith("/admin/billing/services/"))
      return {
        id: other,
        slug: "models",
        name: "Models",
        is_active: true,
        effective_metric: "requests",
        allowance_metrics: [],
        effective: {
          fallback_metric: "requests",
          classes: [],
          resale: { configured: false, applied: false, source: "free" },
          model_prices: [
            {
              model_id: id,
              variant: {},
              lane: "platform_key",
              active_tariff_id: null,
              pending_tariff_id: other,
              available: false,
              blocking_reason: "missing_tariff",
              components: [],
            },
          ],
        },
      };
    if (path.endsWith("/tariffs"))
      return [
        {
          id: other,
          revision: 6,
          service_id: other,
          model_id: id,
          public_model: model.public_id,
          lane: "platform_key",
          variant: {},
          contract: {},
          bounds: {},
          components: [component],
          source: "Private verification source",
          status: "pending",
          sync_error: "Rate sync pending",
          created_at: model.created_at,
        },
      ];
    if (path.includes("/access?"))
      return {
        model_id: id,
        user_id: other,
        revision: 7,
        policy_reason: "explicit_deny",
        model_reason: "disabled",
        flag_enabled: true,
        executable: false,
        route_count: 0,
        route_revision_checked: null,
      };
    return { models: [model], next_after: null };
  });
  mock.post.mockResolvedValue({});
  mock.put.mockResolvedValue(model);
});
afterEach(() => useAuthStore.setState({ user: null }));
describe("platform model surfaces", () => {
  it("hides discovery and sends no request with flag off", () => {
    account(false);
    show(<PlatformModelsPage />);
    expect(screen.queryByText("Platform Models")).not.toBeInTheDocument();
    expect(mock.get).not.toHaveBeenCalled();
    expect(
      getVisibleMainNav(useAuthStore.getState().user).some(
        (n) => n.to === "/models",
      ),
    ).toBe(false);
  });
  it("shows entitled public metadata and exact prices without implying execution readiness", async () => {
    show(<PlatformModelsPage />);
    expect(await screen.findByText("Test model")).toBeInTheDocument();
    expect(screen.getByText("Execution unavailable")).toBeInTheDocument();
    expect(
      screen.getByText("0.000000000001 credits / unit"),
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("private_metric");
    expect(mock.get).toHaveBeenCalledTimes(1);
    expect(mock.get).toHaveBeenCalledWith("/llm/models?limit=50");
  });
  it("paginates with the actual after cursor", async () => {
    mock.get
      .mockResolvedValueOnce({
        models: [publicModel],
        next_after: "nyxid/gpt-test",
      })
      .mockResolvedValueOnce({
        models: [
          { ...publicModel, id: "nyxid/two", display_name: "Second model" },
        ],
        next_after: null,
      });
    show(<PlatformModelsPage />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Load more models" }),
    );
    expect(await screen.findByText("Second model")).toBeInTheDocument();
    expect(mock.get).toHaveBeenCalledWith(
      "/llm/models?limit=50&after=nyxid%2Fgpt-test",
    );
  });
  it("keeps retail prices visible when execution depends on the request", async () => {
    mock.get.mockResolvedValue({
      models: [{ ...publicModel, availability: "request_dependent" }],
      next_after: null,
    });
    show(<PlatformModelsPage />);
    expect(
      await screen.findByText("Availability checked per request"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Execution unavailable")).not.toBeInTheDocument();
    expect(screen.queryByText("Ready for use")).not.toBeInTheDocument();
    expect(
      screen.getByText("0.000000000001 credits / unit"),
    ).toBeInTheDocument();
  });
  it.each([
    [
      false,
      "execution_fleet_not_ready",
      "Execution unavailable: Execution fleet not ready",
    ],
    [true, "", "Execution ready"],
    [
      false,
      "execution_not_implemented",
      "Execution unavailable: Execution not implemented",
    ],
  ])(
    "renders readiness and activation for ready=%s reason=%s",
    async (ready, reason, label) => {
      mock.get.mockResolvedValue({
        ...model,
        execution_ready: ready,
        blocking_reason: reason,
      });
      show(<AdminPlatformModelDetailPage />);
      expect(await screen.findByText(label)).toBeInTheDocument();
      const readiness = screen.getByText(/Required fleet readiness:/);
      for (const capability of model.required_readiness) {
        expect(readiness).toHaveTextContent(capability);
      }
      expect(readiness).not.toHaveTextContent("assistant:platform-models-v1");
      const enable = screen.getByRole("checkbox", {
        name: "Enabled for entitled people",
      });
      if (ready) expect(enable).toBeEnabled();
      else expect(enable).toBeDisabled();
    },
  );
  it("shows missing prices as unavailable, never free", async () => {
    mock.get.mockResolvedValue({
      models: [{ ...publicModel, retail_prices: [] }],
      next_after: null,
    });
    show(<PlatformModelsPage />);
    expect(
      await screen.findByText("Retail prices unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Free")).not.toBeInTheDocument();
  });
  it.each([
    [false, "admin"],
    [true, "operator"],
  ])(
    "keeps admin list read-only when enabled=%s role=%s",
    async (enabled, role) => {
      account(enabled, role);
      show(<AdminPlatformModelsPage />);
      await screen.findByText("Test model");
      expect(
        screen.queryByRole("button", { name: "Create model" }),
      ).not.toBeInTheDocument();
      expect(screen.getByText(/Read-only/)).toBeInTheDocument();
      expect(mock.post).not.toHaveBeenCalled();
    },
  );
  it("retains dirty contract edits after a stale revision conflict", async () => {
    mock.put.mockRejectedValue(
      new ApiError(409, {
        error: "conflict",
        message: "Model changed",
        error_code: 409,
      }),
    );
    show(<AdminPlatformModelDetailPage />);
    const input = await screen.findByLabelText("Display name");
    await userEvent.clear(input);
    await userEvent.type(input, "My edit");
    const save = screen.getByRole("button", { name: "Save model" });
    await waitFor(() => expect(save).toBeEnabled());
    await userEvent.click(save);
    expect(
      await screen.findByText(/Your edits have not been applied/),
    ).toBeInTheDocument();
    expect(input).toHaveValue("My edit");
    expect(mock.put).toHaveBeenCalledWith(
      `/admin/platform-models/${id}`,
      expect.objectContaining({
        expected_revision: 7,
        display_name: "My edit",
      }),
    );
    expect(mock.put).toHaveBeenCalledTimes(1);
    expect(
      screen.getByRole("button", { name: "Discard edits and reload" }),
    ).toBeInTheDocument();
  });
  it("explains deny precedence without executing a model", async () => {
    show(<AdminPlatformModelDetailPage />);
    await userEvent.click(await screen.findByRole("tab", { name: "Access" }));
    expect(screen.getByText(/Any matching deny wins/)).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText("Person ID"), other);
    await userEvent.click(
      screen.getByRole("button", { name: "Explain access" }),
    );
    expect(
      await screen.findByText("Policy: Explicit deny"),
    ).toBeInTheDocument();
    expect(screen.getByText("Can execute: No")).toBeInTheDocument();
    expect(mock.post).not.toHaveBeenCalled();
  });
  it("publishes via billing sync and confirms a revision-fenced retirement", async () => {
    show(<AdminPlatformModelDetailPage />);
    await userEvent.click(
      await screen.findByRole("tab", { name: "Prices & readiness" }),
    );
    await screen.findByText("Revision 6 · Pending · Platform key");
    expect(
      screen.getByText("0.000000000001 credits / unit"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Publish / retry rate sync" }),
    );
    await waitFor(() =>
      expect(mock.post).toHaveBeenCalledWith(
        `/admin/billing/services/${other}/resync`,
        {},
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Retire revision" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Confirm retire revision" }),
    );
    await waitFor(() =>
      expect(mock.post).toHaveBeenCalledWith(
        `/admin/platform-models/${id}/tariffs/${other}/retire`,
        { revision: 7 },
      ),
    );
  });
  it("edits same-model routes privately with CAS", async () => {
    show(<AdminPlatformModelDetailPage />);
    await userEvent.click(
      await screen.findByRole("tab", { name: "Private routes" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Add private route" }),
    );
    await userEvent.type(
      screen.getByLabelText("Credential catalog-service ID"),
      other,
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Publish route revision" }),
      ).toBeEnabled(),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Publish route revision" }),
    );
    await waitFor(() =>
      expect(mock.put).toHaveBeenCalledWith(
        `/admin/platform-models/${id}/routes`,
        expect.objectContaining({
          expected_revision: 7,
          routes: [
            expect.objectContaining({
              credential_service_id: other,
              target: {
                kind: "direct_provider",
                provider: "openai",
                native_model: "gpt-test",
              },
            }),
          ],
        }),
      ),
    );
  });
  it("operator cannot edit the public contract", async () => {
    account(true, "operator");
    show(<AdminPlatformModelDetailPage />);
    expect(await screen.findByLabelText("Display name")).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: "Save model" }),
    ).not.toBeInTheDocument();
  });
});
describe("model editor validation", () => {
  it("explains why Save is disabled before any mutation", async () => {
    show(<AdminPlatformModelDetailPage />);
    await userEvent.type(
      await screen.findByLabelText("Output token limit (optional)"),
      "100000001",
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Enter a positive limit up to 100000000",
    );
    expect(screen.getByRole("button", { name: "Save model" })).toBeDisabled();
    expect(mock.put).not.toHaveBeenCalled();
  });
});
describe("public preview", () => {
  it("contains only customer metadata, never private tariff or routing identity", async () => {
    show(<AdminPlatformModelDetailPage />);
    await userEvent.click(
      await screen.findByRole("tab", { name: "Public preview" }),
    );
    await waitFor(() =>
      expect(screen.queryByText("Loading prices…")).not.toBeInTheDocument(),
    );
    const preview = JSON.parse(document.querySelector("pre")!.textContent!);
    expect(preview.id).toBe("nyxid/gpt-test");
    expect(preview).not.toHaveProperty("billing_service_id");
    expect(preview).not.toHaveProperty("routes");
    expect(preview.retail_prices[0]).toEqual({
      variant: {},
      available: false,
      components: [],
    });
    expect(JSON.stringify(preview)).not.toContain("private_metric");
  });
});
describe("public contract readers", () => {
  it("matches server family, text and capability bounds", () => {
    const valid = modelDefaults(model);
    expect(modelFormSchema.safeParse(valid).success).toBe(true);
    for (const patch of [
      { family: "gemini" },
      { display_name: "a".repeat(161) },
      { description: "a".repeat(2001) },
      { context_tokens: "100000001" },
      { context_tokens: "100", output_tokens: "101" },
    ])
      expect(modelFormSchema.safeParse({ ...valid, ...patch }).success).toBe(
        false,
      );
  });
  it("defaults old-server prices and strips private fields", () => {
    const parsed = publicModelSchema.parse({
      ...publicModel,
      retail_prices: undefined,
      routes: ["private"],
      billing_service_id: other,
    });
    expect(parsed.retail_prices).toEqual([]);
    expect(parsed).not.toHaveProperty("routes");
    expect(parsed).not.toHaveProperty("billing_service_id");
  });
  it("rejects unsafe numeric limits before conversion", () => {
    expect(
      modelFormSchema.safeParse({
        ...modelDefaults(model),
        context_tokens: "9007199254740993",
      }).success,
    ).toBe(false);
  });
});
