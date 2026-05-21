import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type { MfaSetupResponse } from "@/types/api";
import { MfaSetupDialog } from "./mfa-setup-dialog";

const { mockSetupMutateAsync, mockApiPost, mockToastError, mockToastSuccess } =
  vi.hoisted(() => ({
    mockSetupMutateAsync: vi.fn(),
    mockApiPost: vi.fn(),
    mockToastError: vi.fn(),
    mockToastSuccess: vi.fn(),
  }));

// `useMfaSetup` is the enroll-start hook (POST /auth/mfa/setup). The dialog then
// confirms via `api.post("/auth/mfa/confirm", ...)` directly, so mock both.
vi.mock("@/hooks/use-auth", () => ({
  useMfaSetup: () => ({
    mutateAsync: mockSetupMutateAsync,
    isPending: false,
  }),
}));

vi.mock("@/lib/api-client", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/api-client")>(
      "@/lib/api-client",
    );
  return {
    ...actual,
    api: { post: mockApiPost },
  };
});

vi.mock("sonner", () => ({
  toast: { error: mockToastError, success: mockToastSuccess },
}));

// QRCode.toDataURL is async and touches canvas; stub it to a fixed data URL.
vi.mock("qrcode", () => ({
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,QR") },
}));

const setupResponse: MfaSetupResponse = {
  factor_id: "factor-1",
  secret: "JBSWY3DPEHPK3PXP",
  qr_code_url: "otpauth://totp/NyxID:a@b.com?secret=JBSWY3DPEHPK3PXP",
};

function renderDialog() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );
  return render(<MfaSetupDialog open onOpenChange={vi.fn()} />, { wrapper });
}

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("MfaSetupDialog", () => {
  it("TOTP enroll happy path: begin setup -> render secret -> confirm 6-digit code via /auth/mfa/confirm -> show recovery codes", async () => {
    const user = userEvent.setup();
    mockSetupMutateAsync.mockResolvedValue(setupResponse);
    mockApiPost.mockResolvedValue({
      message: "ok",
      recovery_codes: ["AAAA-1111", "BBBB-2222"],
    });

    renderDialog();

    // Step 1: setup -> calls useMfaSetup().mutateAsync to fetch secret/otpauth.
    await user.click(screen.getByRole("button", { name: "Begin Setup" }));
    await waitFor(() => {
      expect(mockSetupMutateAsync).toHaveBeenCalledTimes(1);
    });

    // Step 2: verify step renders the manual-entry secret returned by setup.
    expect(await screen.findByText(setupResponse.secret)).toBeInTheDocument();

    // Step 3: user enters the code and submits; confirm hits /auth/mfa/confirm.
    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "123456",
    );
    await user.click(screen.getByRole("button", { name: "Verify and Enable" }));

    await waitFor(() => {
      expect(mockApiPost).toHaveBeenCalledWith("/auth/mfa/confirm", {
        code: "123456",
      });
    });

    // Step 4: success advances to the recovery-codes step.
    expect(await screen.findByText("AAAA-1111")).toBeInTheDocument();
    expect(screen.getByText("BBBB-2222")).toBeInTheDocument();
    expect(mockToastSuccess).toHaveBeenCalledWith("MFA enabled successfully");
  });

  it("invalid-code rejection: confirm rejects -> error shown, dialog stays on verify step", async () => {
    const user = userEvent.setup();
    mockSetupMutateAsync.mockResolvedValue(setupResponse);
    mockApiPost.mockRejectedValue(
      new ApiError(400, {
        error: "invalid_code",
        error_code: 2003,
        message: "Invalid verification code",
      }),
    );

    renderDialog();

    await user.click(screen.getByRole("button", { name: "Begin Setup" }));
    await screen.findByText(setupResponse.secret);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "000000",
    );
    await user.click(screen.getByRole("button", { name: "Verify and Enable" }));

    // ApiError.message is surfaced as the form root error.
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Invalid verification code",
    );
    // Dialog stays on the verify step (no recovery code, secret still visible).
    expect(screen.getByText(setupResponse.secret)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Verify and Enable" }),
    ).toBeInTheDocument();
    expect(mockToastSuccess).not.toHaveBeenCalled();
  });
});
