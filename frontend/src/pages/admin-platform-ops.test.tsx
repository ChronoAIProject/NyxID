import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { adminPricing } from "@/schemas/__fixtures__/platform-ops-builders";
import type {
  AdminPlatformOperationList,
  AdminPlatformProvider,
} from "@/schemas/platform-ops";
import { AdminPlatformOpsPage } from "./admin-platform-ops";

const {
  mockDeleteCredential,
  mockDemote,
  mockPromote,
  mockRefetchOperations,
  mockRefetchProviders,
  mockSetCredential,
  mockToastError,
  mockToastSuccess,
  mockUpdate,
} = vi.hoisted(() => ({
  mockDeleteCredential: vi.fn(),
  mockDemote: vi.fn(),
  mockPromote: vi.fn(),
  mockRefetchOperations: vi.fn(),
  mockRefetchProviders: vi.fn(),
  mockSetCredential: vi.fn(),
  mockToastError: vi.fn(),
  mockToastSuccess: vi.fn(),
  mockUpdate: vi.fn(),
}));

const providerId = "00000000-0000-4000-8000-000000000010";

const operations: AdminPlatformOperationList = {
  operations: [
    {
      operation_id: "00000000-0000-4000-8000-000000000001",
      catalog_service_id: providerId,
      provider_slug: "platform-openai",
      provider_name: "OpenAI",
      operation_name: "Create response",
      enabled: true,
      kind: {
        type: "endpoint",
        method: "POST",
        path_template: "/v1/responses",
        name: "Create response",
        description: "Create a model response.",
      },
      limits: {
        per_request: { type: "endpoint" },
        per_user_per_day: 100,
      },
      pricing: adminPricing({
        billable: true,
        metric: "input_tokens",
        price_per_unit: "0.000002",
        secondary: {
          metric: "output_tokens",
          price_per_unit: "0.000008",
          lago_metric_code: "platform_op_openai_output",
        },
        display:
          "0.000002 credits per input token + 0.000008 credits per output token",
        lago_metric_code: "platform_op_openai_input",
        sync_status: "failed",
        sync_error: "Lago rejected the charge",
      }),
      created_at: "2026-08-25T09:00:00Z",
      created_by: "admin-1",
      updated_at: "2026-08-25T09:00:00Z",
      updated_by: "admin-1",
    },
    {
      operation_id: "00000000-0000-4000-8000-000000000002",
      catalog_service_id: providerId,
      provider_slug: "platform-openai",
      provider_name: "OpenAI",
      operation_name: "Speak",
      enabled: true,
      kind: {
        type: "constrained",
        op: "speak",
        config: {
          type: "speak",
          allowed_voice_ids: ["voice-a"],
          model_id: "eleven_multilingual_v2",
          max_calls_per_user_per_day: 50,
        },
      },
      limits: {
        per_request: { type: "speak", max_chars: 1_000 },
        per_user_per_day: 50,
      },
      pricing: adminPricing({
        billable: true,
        metric: "characters",
        price_per_unit: "0.0001",
        display: "0.0001 credits per character",
        lago_metric_code: "platform_op_speak_characters",
        sync_status: "synced",
      }),
      created_at: "2026-08-25T09:00:00Z",
      created_by: "admin-1",
      updated_at: "2026-08-25T09:00:00Z",
      updated_by: "admin-1",
    },
  ],
};

const provider: AdminPlatformProvider = {
  catalog_service_id: providerId,
  catalog_service_slug: "platform-openai",
  catalog_service_name: "OpenAI",
  catalog_service_active: true,
  eligible: true,
  eligibility_reason: null,
  promoted: false,
  promoted_at: null,
  promoted_by: null,
  vendor_terms_accepted_at: null,
  vendor_terms_accepted_by: null,
  credential: {
    configured: false,
    id: null,
    auth_method: null,
    auth_key_name: null,
    created_at: null,
    updated_at: null,
  },
  enabled_operation_count: 2,
};

let currentProvider = provider;

vi.mock("@/hooks/use-platform-ops", () => ({
  usePlatformOperations: () => ({
    data: operations,
    error: null,
    isLoading: false,
    refetch: mockRefetchOperations,
  }),
  usePlatformProviders: () => ({
    data: { providers: [currentProvider] },
    error: null,
    isLoading: false,
    refetch: mockRefetchProviders,
  }),
  useUpdatePlatformOperation: () => ({
    isPending: false,
    mutateAsync: mockUpdate,
  }),
  usePromotePlatformProvider: () => ({
    isPending: false,
    mutateAsync: mockPromote,
  }),
  useDemotePlatformProvider: () => ({
    isPending: false,
    mutateAsync: mockDemote,
  }),
  useSetPlatformCredential: () => ({
    isPending: false,
    mutateAsync: mockSetCredential,
  }),
  useDeletePlatformCredential: () => ({
    isPending: false,
    mutateAsync: mockDeleteCredential,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: mockToastError, success: mockToastSuccess },
}));

describe("AdminPlatformOpsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    currentProvider = provider;
    mockUpdate.mockResolvedValue(operations.operations[0]);
    mockPromote.mockResolvedValue({ ...provider, promoted: true });
    mockSetCredential.mockResolvedValue({
      ...provider,
      promoted: true,
      credential: {
        ...provider.credential,
        configured: true,
        id: "00000000-0000-4000-8000-000000000020",
      },
    });
  });

  it("renders provider groups and only the eight truthful admin columns", () => {
    render(<AdminPlatformOpsPage />);

    for (const heading of [
      "Provider",
      "Operation",
      "Kind",
      "Enabled",
      "Metric",
      "Price",
      "Limits",
      "Billing sync",
    ]) {
      expect(
        screen.getByRole("columnheader", { name: heading }),
      ).toBeInTheDocument();
    }
    expect(screen.getAllByText("OpenAI").length).toBeGreaterThan(1);
    expect(screen.getAllByText("POST /v1/responses").length).toBeGreaterThan(0);
    expect(screen.getAllByText("+ output tokens").length).toBeGreaterThan(0);
    expect(
      screen.getAllByText(
        "0.000002 credits per input token + 0.000008 credits per output token",
      ).length,
    ).toBeGreaterThan(0);
    expect(screen.getAllByText("failed").length).toBeGreaterThan(0);

    expect(screen.queryByText("Usage today")).not.toBeInTheDocument();
    expect(screen.queryByText("Credits spent")).not.toBeInTheDocument();
    expect(screen.queryByText("Health")).not.toBeInTheDocument();
  });

  it("opens endpoint fields and keeps UUID saves dirty-gated", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    const operationRow = screen
      .getAllByText("POST /v1/responses")[0]
      ?.closest("tr");
    expect(operationRow).not.toBeNull();
    await user.click(operationRow as HTMLElement);

    const drawer = await screen.findByRole("dialog");
    expect(within(drawer).getByLabelText("Method")).toHaveValue("POST");
    expect(within(drawer).getByLabelText("Canonical Path")).toHaveValue(
      "/v1/responses",
    );
    expect(within(drawer).getByText("Lago rejected the charge")).toBeVisible();
    expect(within(drawer).getByLabelText("Secondary Metric")).toHaveTextContent(
      "output tokens",
    );

    const save = within(drawer).getByRole("button", { name: "Save changes" });
    expect(save).toBeDisabled();
    fireEvent.change(within(drawer).getByLabelText("Canonical Path"), {
      target: { value: "/v1/responses/{response_id}" },
    });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({
        operationId: operations.operations[0]?.operation_id,
        data: expect.objectContaining({
          kind: expect.objectContaining({
            kind: "endpoint",
            method: "POST",
            path_template: "/v1/responses/{response_id}",
          }),
        }),
      }),
    );
  });

  it("keeps the constrained speak daily cap synchronized on save", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    const speakRow = screen
      .getAllByText("Speak")
      .find((element) => element.closest("tr"))
      ?.closest("tr");
    expect(speakRow).not.toBeNull();
    await user.click(speakRow as HTMLElement);

    const drawer = await screen.findByRole("dialog");
    fireEvent.change(within(drawer).getByLabelText("Daily Calls Per Owner"), {
      target: { value: "75" },
    });
    await user.click(
      within(drawer).getByRole("button", { name: "Save changes" }),
    );

    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({
        operationId: operations.operations[1]?.operation_id,
        data: expect.objectContaining({
          kind: expect.objectContaining({
            kind: "constrained",
            op: "speak",
            config: expect.objectContaining({
              max_calls_per_user_per_day: 75,
            }),
          }),
          limits: expect.objectContaining({ per_user_per_day: 75 }),
        }),
      }),
    );
  });

  it("requires explicit vendor-terms acceptance before promotion", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Provider" }));

    const drawer = await screen.findByRole("dialog");
    const promote = within(drawer).getByRole("button", {
      name: "Promote provider",
    });
    expect(promote).toBeDisabled();
    await user.click(within(drawer).getByLabelText("Accept vendor terms"));
    expect(promote).toBeEnabled();
    await user.click(promote);

    await waitFor(() => expect(mockPromote).toHaveBeenCalledWith(providerId));
  });

  it("submits credential material write-only and clears the password field", async () => {
    currentProvider = {
      ...provider,
      promoted: true,
      promoted_at: "2026-08-25T10:00:00Z",
      promoted_by: "admin-1",
      vendor_terms_accepted_at: "2026-08-25T10:00:00Z",
      vendor_terms_accepted_by: "admin-1",
    };
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Provider" }));

    const drawer = await screen.findByRole("dialog");
    const credential = within(drawer).getByLabelText("Credential");
    await user.type(credential, "vendor-secret");
    await user.click(within(drawer).getByRole("button", { name: "Configure" }));

    await waitFor(() =>
      expect(mockSetCredential).toHaveBeenCalledWith({
        providerId,
        data: { credential: "vendor-secret" },
      }),
    );
    await waitFor(() => expect(credential).toHaveValue(""));
    expect(screen.queryByText("vendor-secret")).not.toBeInTheDocument();
  });
});
