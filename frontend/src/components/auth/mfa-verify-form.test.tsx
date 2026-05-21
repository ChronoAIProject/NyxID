import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { MfaVerifyForm } from "./mfa-verify-form";

const { mockVerifyMutateAsync, mockNavigate, mockClearMfaState, storeState } =
  vi.hoisted(() => ({
    mockVerifyMutateAsync: vi.fn(),
    mockNavigate: vi.fn(),
    mockClearMfaState: vi.fn(),
    storeState: { mfaToken: "mfa-session-token" as string | null },
  }));

vi.mock("@/hooks/use-auth", () => ({
  useMfaVerify: () => ({
    mutateAsync: mockVerifyMutateAsync,
    isPending: false,
  }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

// The form reads `mfaToken` and `clearMfaState` via auth-store selectors.
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (
    selector: (s: {
      mfaToken: string | null;
      clearMfaState: () => void;
    }) => unknown,
  ) =>
    selector({
      mfaToken: storeState.mfaToken,
      clearMfaState: mockClearMfaState,
    }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  storeState.mfaToken = "mfa-session-token";
});

describe("MfaVerifyForm", () => {
  it("submits code -> calls useMfaVerify with the code + stored mfa_token, then navigates to /dashboard", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);

    render(<MfaVerifyForm />);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "123456",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(mockVerifyMutateAsync).toHaveBeenCalledWith({
        code: "123456",
        mfa_token: "mfa-session-token",
      });
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
  });

  it("redirects to a same-origin returnTo URL instead of /dashboard on success", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    const returnTo = `${window.location.origin}/settings`;

    render(<MfaVerifyForm returnTo={returnTo} />);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "654321",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledWith(returnTo);
    });
    // Trusted returnTo wins: no router navigation to the dashboard.
    expect(mockNavigate).not.toHaveBeenCalled();
    assignSpy.mockRestore();
  });

  it("rejects an untrusted-origin returnTo: navigates to /dashboard and never calls window.location.assign (open-redirect guard)", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    // A returnTo whose origin is neither the frontend nor the backend must
    // be discarded -- the user lands on /dashboard, not the attacker host.
    const returnTo = "https://evil.example/x";

    render(<MfaVerifyForm returnTo={returnTo} />);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "654321",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
    });
    expect(assignSpy).not.toHaveBeenCalled();
    assignSpy.mockRestore();
  });

  it("does not verify when mfaToken is null (the !mfaToken submit guard)", async () => {
    const user = userEvent.setup();
    storeState.mfaToken = null;

    render(<MfaVerifyForm />);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "123456",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    // The submit handler short-circuits on `!mfaToken` before mutating, so
    // a valid code with no session token never reaches the verify mutation.
    expect(mockVerifyMutateAsync).not.toHaveBeenCalled();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("shows the ApiError message when verification is rejected and does not navigate", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockRejectedValue(
      new ApiError(401, {
        error: "invalid_code",
        error_code: 2003,
        message: "Invalid code",
      }),
    );

    render(<MfaVerifyForm />);

    await user.type(
      screen.getByLabelText("Enter 6-digit verification code"),
      "111111",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("Invalid code");
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("'Back to Login' clears MFA state without verifying", async () => {
    const user = userEvent.setup();

    render(<MfaVerifyForm />);

    await user.click(screen.getByRole("button", { name: "Back to Login" }));

    expect(mockClearMfaState).toHaveBeenCalledTimes(1);
    expect(mockVerifyMutateAsync).not.toHaveBeenCalled();
  });
});
