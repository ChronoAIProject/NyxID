import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import { AuthFlow } from "./auth-flow";

const { config, mockNavigate, mockOpenExternal, loginFn, registerFn, toastFns } =
  vi.hoisted(() => ({
    config: {
      value: {
        invite_code_required: true,
        email_auth_enabled: true,
        social_providers: ["google", "github"] as string[],
      } as Record<string, unknown> | undefined,
    },
    mockNavigate: vi.fn(),
    mockOpenExternal: vi.fn(),
    loginFn: vi.fn(),
    registerFn: vi.fn(),
    toastFns: { info: vi.fn(), error: vi.fn(), success: vi.fn() },
  }));

vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: config.value }),
}));

vi.mock("@/hooks/use-auth", () => ({
  useLogin: () => ({ mutateAsync: loginFn, isPending: false }),
  useRegister: () => ({ mutateAsync: registerFn, isPending: false }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
  Link: ({
    children,
    to,
  }: {
    readonly children: React.ReactNode;
    readonly to: string;
  }) => <a href={to}>{children}</a>,
}));

vi.mock("@/lib/navigation", () => ({
  openExternal: (url: string) => mockOpenExternal(url),
}));

vi.mock("sonner", () => ({ toast: toastFns }));

beforeEach(() => {
  vi.clearAllMocks();
  config.value = {
    invite_code_required: true,
    email_auth_enabled: true,
    social_providers: ["google", "github"],
  };
});

afterEach(() => {
  vi.useRealTimers();
});

async function fillLoginAndSubmit(user: ReturnType<typeof userEvent.setup>) {
  await user.type(
    screen.getByPlaceholderText("you@example.com"),
    "user@example.com",
  );
  await user.type(
    screen.getByPlaceholderText("Enter your password"),
    "correct horse",
  );
  await user.click(screen.getByRole("button", { name: "Sign in" }));
}

describe("AuthFlow — login", () => {
  it("submits credentials and navigates to the dashboard on success", async () => {
    loginFn.mockResolvedValue({ mfaRequired: false });
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={0} />);

    await fillLoginAndSubmit(user);

    await waitFor(() => {
      expect(loginFn).toHaveBeenCalledWith({
        email: "user@example.com",
        password: "correct horse",
      });
    });
    expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" });
  });

  it("redirects to a same-origin return_to instead of the dashboard", async () => {
    loginFn.mockResolvedValue({ mfaRequired: false });
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    const returnTo = `${window.location.origin}/keys`;
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={0} returnTo={returnTo} />);

    await fillLoginAndSubmit(user);

    await waitFor(() => expect(assignSpy).toHaveBeenCalledWith(returnTo));
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("ignores an untrusted-origin return_to and falls back to the dashboard", async () => {
    loginFn.mockResolvedValue({ mfaRequired: false });
    const assignSpy = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    const user = userEvent.setup();
    // Open-redirect guard: an off-origin return_to must NOT be assigned.
    render(<AuthFlow initialPanel={0} returnTo="https://evil.example/keys" />);

    await fillLoginAndSubmit(user);

    await waitFor(() =>
      expect(mockNavigate).toHaveBeenCalledWith({ to: "/dashboard" }),
    );
    expect(assignSpy).not.toHaveBeenCalled();
  });

  it("shows the server error message and does not navigate on bad credentials", async () => {
    loginFn.mockRejectedValue(
      new ApiError(401, {
        error: "invalid_credentials",
        error_code: 1001,
        message: "Invalid email or password",
      }),
    );
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={0} />);

    await fillLoginAndSubmit(user);

    expect(
      await screen.findByText("Invalid email or password"),
    ).toBeInTheDocument();
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it("does not navigate when the login response signals MFA is required", async () => {
    loginFn.mockResolvedValue({ mfaRequired: true });
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={0} />);

    await fillLoginAndSubmit(user);

    await waitFor(() => expect(loginFn).toHaveBeenCalled());
    expect(mockNavigate).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("maps a known social-auth error to a friendly message", () => {
    render(<AuthFlow initialPanel={0} socialError="social_auth_conflict" />);
    expect(screen.getByTestId("social-error")).toHaveTextContent(
      /already linked elsewhere/i,
    );
  });

  it("falls back to a generic message for an unknown social-auth error", () => {
    render(<AuthFlow initialPanel={0} socialError="totally_unknown" />);
    expect(screen.getByTestId("social-error")).toHaveTextContent(
      "Social sign-in failed. Please try again.",
    );
  });

  it("maps invite_code_already_redeemed to the already-redeemed message", () => {
    render(
      <AuthFlow
        initialPanel={0}
        socialError="invite_code_already_redeemed"
      />,
    );
    expect(screen.getByTestId("social-error")).toHaveTextContent(
      "This invite code has already been redeemed with this account.",
    );
  });

  it("hides the email/password form when email auth is disabled", () => {
    config.value = {
      invite_code_required: true,
      email_auth_enabled: false,
      social_providers: ["google"],
    };
    render(<AuthFlow initialPanel={0} />);

    expect(
      screen.queryByPlaceholderText("Enter your password"),
    ).not.toBeInTheDocument();
    // Social providers remain the only sign-in route.
    expect(
      screen.getByRole("button", { name: /Continue with Google/i }),
    ).toBeInTheDocument();
  });
});

describe("AuthFlow — register", () => {
  it("blocks the email step and shows the invite gate when no code is entered", async () => {
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={1} />);

    await user.click(screen.getByRole("button", { name: /Continue with Email/i }));

    expect(
      screen.getByText("An invite code is required to use NyxID at this time."),
    ).toBeInTheDocument();
    expect(registerFn).not.toHaveBeenCalled();
  });

  it("submits the registration with the normalized invite code on the happy path", async () => {
    registerFn.mockResolvedValue({ message: "Check your email." });
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={1} initialInviteCode="nyx-abcd1234" />);

    await user.type(screen.getByPlaceholderText("John Doe"), "Ada Lovelace");
    await user.type(
      screen.getByPlaceholderText("you@example.com"),
      "ada@example.com",
    );
    await user.type(screen.getByPlaceholderText("Min 8 characters"), "Hunter22");
    await user.type(
      screen.getByPlaceholderText("Re-enter your password"),
      "Hunter22",
    );
    await user.click(screen.getByRole("button", { name: "Create Account" }));

    await waitFor(() => {
      expect(registerFn).toHaveBeenCalledWith({
        display_name: "Ada Lovelace",
        email: "ada@example.com",
        password: "Hunter22",
        invite_code: "NYX-ABCD1234",
      });
    });
    expect(toastFns.info).toHaveBeenCalledWith("Check your email.");
    // The success path slides back to the login view via a timed fade —
    // flush those pending timers inside act() so they don't leak.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 500));
    });
  });

  it("surfaces an email-already-registered error and stays on the form", async () => {
    registerFn.mockRejectedValue(
      new ApiError(409, {
        error: "email_exists",
        error_code: 1002,
        message: "An account with this email already exists",
      }),
    );
    const user = userEvent.setup();
    render(<AuthFlow initialPanel={1} initialInviteCode="NYX-ABCD1234" />);

    await user.type(screen.getByPlaceholderText("John Doe"), "Ada Lovelace");
    await user.type(
      screen.getByPlaceholderText("you@example.com"),
      "ada@example.com",
    );
    await user.type(screen.getByPlaceholderText("Min 8 characters"), "Hunter22");
    await user.type(
      screen.getByPlaceholderText("Re-enter your password"),
      "Hunter22",
    );
    await user.click(screen.getByRole("button", { name: "Create Account" }));

    expect(
      await screen.findByText("An account with this email already exists"),
    ).toBeInTheDocument();
    expect(toastFns.info).not.toHaveBeenCalled();
  });
});
