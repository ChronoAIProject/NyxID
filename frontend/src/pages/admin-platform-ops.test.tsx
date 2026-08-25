import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PlatformOperationList } from "@/schemas/platform-ops";
import { AdminPlatformOpsPage } from "./admin-platform-ops";

const { mockMutateAsync, mockRefetch, mockToastError, mockToastSuccess } =
  vi.hoisted(() => ({
    mockMutateAsync: vi.fn(),
    mockRefetch: vi.fn(),
    mockToastError: vi.fn(),
    mockToastSuccess: vi.fn(),
  }));

const operations: PlatformOperationList = {
  operations: [
    {
      op: "x_search",
      enabled: false,
      vendor_service_slug: "platform-x",
      config: { type: "x_search", max_results_cap: 10 },
      updated_at: null,
      updated_by: null,
    },
    {
      op: "speak",
      enabled: false,
      vendor_service_slug: "platform-elevenlabs",
      config: {
        type: "speak",
        allowed_voice_ids: ["voice-a"],
        max_chars: 1000,
        model_id: "eleven_multilingual_v2",
      },
      updated_at: null,
      updated_by: null,
    },
    {
      op: "call_and_say",
      enabled: false,
      vendor_service_slug: "platform-twilio",
      config: {
        type: "call_and_say",
        allowed_destination_prefixes: ["+65"],
        max_message_chars: 500,
        voice: "alice",
        max_calls_per_user_per_day: 3,
        account_sid: `AC${"a".repeat(32)}`,
        call_from: "+6512345678",
      },
      updated_at: null,
      updated_by: null,
    },
  ],
};

vi.mock("@/hooks/use-platform-ops", () => ({
  usePlatformOperations: () => ({
    data: operations,
    error: null,
    isLoading: false,
    refetch: mockRefetch,
  }),
  useUpdatePlatformOperation: () => ({
    isPending: false,
    mutateAsync: mockMutateAsync,
  }),
}));

vi.mock("sonner", () => ({
  toast: { error: mockToastError, success: mockToastSuccess },
}));

describe("AdminPlatformOpsPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockMutateAsync.mockImplementation(
      async ({ op, data }: { op: string; data: Record<string, unknown> }) => ({
        op,
        ...data,
        updated_at: "2026-08-25T10:00:00Z",
        updated_by: "admin-1",
      }),
    );
  });

  it("renders one typed form for each fixed operation", () => {
    render(<AdminPlatformOpsPage />);

    expect(screen.getByText("X Search")).toBeInTheDocument();
    expect(screen.getByText("Speak")).toBeInTheDocument();
    expect(screen.getByText("Call and Say")).toBeInTheDocument();
    expect(screen.getByLabelText("Maximum Results")).toHaveValue(10);
    expect(screen.getByLabelText("Allowed Voice IDs")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Allowed Destination Prefixes"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Account SID")).toHaveValue(
      `AC${"a".repeat(32)}`,
    );
  });

  it("keeps saves dirty-gated and submits the typed operation payload", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    const save = screen.getByRole("button", { name: "Save X Search" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Maximum Results"), {
      target: { value: "12" },
    });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);

    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith({
        op: "x_search",
        data: {
          enabled: false,
          vendor_service_slug: "platform-x",
          config: { type: "x_search", max_results_cap: 12 },
        },
      }),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith(
      "X Search configuration saved",
    );
  });

  it("edits allowlists as chips instead of raw JSON", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    const voiceInput = screen.getByLabelText("Allowed Voice IDs");

    await user.type(voiceInput, "voice-b{Enter}");

    expect(screen.getByText("voice-b")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Remove voice ID voice-b" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: /json/i })).toBeNull();
  });
});
