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

const { mockCancel } = vi.hoisted(() => ({ mockCancel: vi.fn() }));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...await importOriginal<typeof import("@/lib/api-client")>(),
  apiClient: mockCancel,
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
  it("uses only the HttpOnly challenge in cookie mode even with a stale password challenge", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    render(<MfaVerifyForm cookieChallenge />);
    await user.click(screen.getByRole("button", { name: "Use a recovery code" }));
    expect(screen.getByLabelText("Recovery code")).toHaveAttribute("inputmode", "text");
    await user.type(screen.getByLabelText("Recovery code"), "R3c-v_12");
    await user.click(screen.getByRole("button", { name: "Verify" }));
    await waitFor(() => expect(mockVerifyMutateAsync).toHaveBeenCalledWith({ code: "R3c-v_12" }));
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/assistant" });
  });

  it("returns an expired social challenge to sign-in with a clear status", async () => {
    const user = userEvent.setup();
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
    mockVerifyMutateAsync.mockRejectedValue(new ApiError(401, { error: "authentication_failed", error_code: 2001, message: "Expired", details: { reason: "mfa_challenge_expired" } }));
    render(<MfaVerifyForm cookieChallenge />);
    await user.type(screen.getByLabelText("Authenticator code"), "123456");
    await user.click(screen.getByRole("button", { name: "Verify" }));
    await waitFor(() => expect(assign).toHaveBeenCalledWith("/login?mfa=expired"));
    expect(mockClearMfaState).toHaveBeenCalled();
    assign.mockRestore();
  });

  it("cancels the cookie challenge before leaving the form", async () => {
    const user = userEvent.setup();
    mockCancel.mockResolvedValue({ cancelled: true });
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
    render(<MfaVerifyForm cookieChallenge />);
    await user.click(screen.getByRole("button", { name: "Back to Login" }));
    await waitFor(() => expect(mockCancel).toHaveBeenCalledWith("/auth/mfa/cancel", { method: "POST" }));
    expect(assign).toHaveBeenCalledWith("/login");
    assign.mockRestore();
  });
  it.each(["/oauth/authorize?state=resume", "/login/device?code=ABCD"])("preserves trusted returnTo on cookie cancellation: %s", async (returnTo) => {
    const user = userEvent.setup();
    mockCancel.mockResolvedValue({ cancelled: true });
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
    render(<MfaVerifyForm cookieChallenge returnTo={returnTo} />);
    await user.click(screen.getByRole("button", { name: "Back to Login" }));
    await waitFor(() => expect(assign).toHaveBeenCalled());
    const target = new URL(assign.mock.calls[0]![0].toString(), window.location.origin);
    expect(target.pathname).toBe("/login");
    expect(target.searchParams.get("return_to")).toBe(new URL(returnTo, window.location.origin).href);
    assign.mockRestore();
  });

  it("resets password MFA in app without navigation or a reload", async () => {
    const user = userEvent.setup();
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
    render(<MfaVerifyForm returnTo="/oauth/authorize?state=resume" />);
    await user.click(screen.getByRole("button", { name: "Back to Login" }));
    expect(mockClearMfaState).toHaveBeenCalledOnce();
    expect(assign).not.toHaveBeenCalled();
    expect(mockNavigate).not.toHaveBeenCalled();
    assign.mockRestore();
  });

  it("submits code -> calls useMfaVerify with the code + stored mfa_token, then navigates to /assistant", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);

    render(<MfaVerifyForm />);

    await user.type(
      screen.getByLabelText("Authenticator code"),
      "123456",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(mockVerifyMutateAsync).toHaveBeenCalledWith({
        code: "123456",
        mfa_token: "mfa-session-token",
      });
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/assistant" });
  });

  it("submits an eight-character recovery code with the existing challenge", async () => {
    const user = userEvent.setup(); mockVerifyMutateAsync.mockResolvedValue(undefined);
    render(<MfaVerifyForm />);
    await user.type(screen.getByLabelText("Authenticator code"), "R3c-v_12");
    await user.click(screen.getByRole("button", { name: "Verify" }));
    await waitFor(() => expect(mockVerifyMutateAsync).toHaveBeenCalledWith({ code: "R3c-v_12", mfa_token: "mfa-session-token" }));
  });

  it("submits the complete 32-hex recovery code", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    render(<MfaVerifyForm />);
    const code = "9b7130d2e5a6417780c9fd2e54b602ad";
    await user.type(screen.getByLabelText("Authenticator code"), code);
    await user.click(screen.getByRole("button", { name: "Verify" }));
    await waitFor(() => expect(mockVerifyMutateAsync).toHaveBeenCalledWith({
      code, mfa_token: "mfa-session-token",
    }));
  });

  it.each([
    "/login/agent-key?user_code=ABCDEFGH",
    "/dashboard?explicit=1",
    "/oauth/authorize?client_id=app&state=review&scope=openid",
  ])("preserves the explicit same-origin returnTo %s after MFA", async (path) => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    const returnTo = `${window.location.origin}${path}`;

    render(<MfaVerifyForm returnTo={returnTo} />);

    await user.type(
      screen.getByLabelText("Authenticator code"),
      "654321",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(assignSpy).toHaveBeenCalledWith(returnTo);
    });
    // Trusted returnTo wins: no router navigation to the Assistant.
    expect(mockNavigate).not.toHaveBeenCalled();
    assignSpy.mockRestore();
  });

  it("rejects an untrusted-origin returnTo: navigates to /assistant and never calls window.location.assign (open-redirect guard)", async () => {
    const user = userEvent.setup();
    mockVerifyMutateAsync.mockResolvedValue(undefined);
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => undefined);
    // A protocol-relative returnTo must be discarded -- the user lands on
    // /assistant, not the attacker host.
    const returnTo = "//evil.example/x";

    render(<MfaVerifyForm returnTo={returnTo} />);

    await user.type(
      screen.getByLabelText("Authenticator code"),
      "654321",
    );
    await user.click(screen.getByRole("button", { name: "Verify" }));

    await waitFor(() => {
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/assistant" });
    });
    expect(assignSpy).not.toHaveBeenCalled();
    assignSpy.mockRestore();
  });

  it("does not verify when mfaToken is null (the !mfaToken submit guard)", async () => {
    const user = userEvent.setup();
    storeState.mfaToken = null;

    render(<MfaVerifyForm />);

    await user.type(
      screen.getByLabelText("Authenticator code"),
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
      screen.getByLabelText("Authenticator code"),
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
