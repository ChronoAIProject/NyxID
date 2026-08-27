import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
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
      op: "speak",
      enabled: false,
      vendor_service_slug: "platform-elevenlabs",
      config: {
        type: "speak",
        allowed_voice_ids: ["voice-a"],
        max_chars: 1_000,
        model_id: "eleven_multilingual_v2",
      },
      updated_at: null,
      updated_by: null,
      vendor_service_id: "platform-elevenlabs-id",
      pricing: {
        billable: false,
        credits_per_call: null,
        metric: "requests",
      },
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
      vendor_service_id: "platform-twilio-id",
      pricing: {
        billable: true,
        credits_per_call: null,
        metric: "requests",
      },
    },
    {
      op: "flight_search",
      enabled: false,
      vendor_service_slug: "platform-duffel",
      config: {
        type: "flight_search",
        max_offers_cap: 10,
        max_searches_per_user_per_day: 20,
      },
      updated_at: null,
      updated_by: null,
      vendor_service_id: "platform-duffel-id",
      pricing: {
        billable: true,
        credits_per_call: "0.5",
        metric: "requests",
      },
    },
  ],
};

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { readonly children: ReactNode; readonly to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

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

  it("renders one typed form for each remaining constrained operation", () => {
    render(<AdminPlatformOpsPage />);

    expect(screen.queryByText("X Search")).not.toBeInTheDocument();
    expect(screen.getByText("Speak")).toBeInTheDocument();
    expect(screen.getByText("Call and Say")).toBeInTheDocument();
    expect(screen.getByText("Flight Search")).toBeInTheDocument();
    expect(screen.getByLabelText("Allowed Voice IDs")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Allowed Destination Prefixes"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Maximum Offers")).toHaveValue(10);
    expect(screen.getByText("0.5 credits per call")).toBeInTheDocument();
    expect(
      screen.getAllByRole("link", { name: /edit service/i }),
    ).toHaveLength(3);
  });

  it("keeps saves dirty-gated and submits the typed operation payload", async () => {
    render(<AdminPlatformOpsPage />);
    const user = userEvent.setup();
    const save = screen.getByRole("button", { name: "Save Speak" });
    expect(save).toBeDisabled();

    fireEvent.change(screen.getByLabelText("Maximum Characters"), {
      target: { value: "1200" },
    });
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);

    await waitFor(() =>
      expect(mockMutateAsync).toHaveBeenCalledWith({
        op: "speak",
        data: {
          enabled: false,
          vendor_service_slug: "platform-elevenlabs",
          config: {
            type: "speak",
            allowed_voice_ids: ["voice-a"],
            max_chars: 1_200,
            model_id: "eleven_multilingual_v2",
          },
        },
      }),
    );
    expect(mockToastSuccess).toHaveBeenCalledWith(
      "Speech configuration saved",
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
