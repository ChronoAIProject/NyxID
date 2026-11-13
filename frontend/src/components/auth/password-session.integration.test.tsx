import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { AuthFlow } from "./auth-flow";

const { navigate, openExternal } = vi.hoisted(() => ({
  navigate: vi.fn(),
  openExternal: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@/lib/navigation", () => ({ openExternal }));
vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({
    data: {
      invite_code_required: true,
      email_auth_enabled: true,
      social_providers: ["google", "github"],
    },
  }),
}));
vi.mock("qrcode", () => ({
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,qr") },
}));

const passwordAccount: User = {
  id: "password-person",
  email: "password@example.com",
  display_name: "Password person",
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-01-01T00:00:00Z",
  capabilities: { enabled_features: ["assistant:nyxagent-engine"] },
};

let finishProfile: (response: Response) => void;
let client: QueryClient;

beforeEach(() => {
  vi.clearAllMocks();
  localStorage.clear();
  useAuthStore.getState().setUser(null);
  useAuthStore.setState({
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (url) => {
      if (url === "/api/v1/auth/login") {
        return new Response(JSON.stringify({ user_id: passwordAccount.id }));
      }
      if (url === "/api/v1/users/me") {
        return new Promise<Response>((resolve) => {
          finishProfile = resolve;
        });
      }
      if (url === "/api/v1/auth/device/request") {
        return new Response(
          JSON.stringify({
            device_code: "nyx_adc_test",
            user_code: "ABCD-EFGH",
            verification_uri: "https://id.example/login/device",
            verification_uri_complete:
              "https://id.example/login/device?user_code=ABCD-EFGH",
            expires_in: 600,
            interval: 1,
          }),
        );
      }
      throw new Error(`Unexpected request: ${String(url)}`);
    }),
  );
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.unstubAllGlobals();
  useAuthStore.getState().setUser(null);
  useAuthStore.setState({
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
});

it.each([200, 503])(
  "blocks parallel sign-in methods until the password profile settles (%s)",
  async (status) => {
    const user = userEvent.setup();
    render(
      <QueryClientProvider client={client}>
        <AuthFlow />
      </QueryClientProvider>,
    );
    await user.type(
      screen.getByPlaceholderText("you@example.com"),
      passwordAccount.email,
    );
    await user.type(
      screen.getByPlaceholderText("Enter your password"),
      "test password",
    );
    await user.click(screen.getByRole("button", { name: "Sign in" }));
    await waitFor(() =>
      expect(fetch).toHaveBeenCalledWith("/api/v1/users/me", expect.anything()),
    );

    const app = screen.getByRole("button", {
      name: "Continue with the NyxID app",
    });
    const google = screen.getByRole("button", { name: "Continue with Google" });
    const github = screen.getByRole("button", { name: "Continue with GitHub" });
    const signUp = screen.getByRole("button", { name: "Sign up" });
    for (const method of [app, google, github, signUp]) {
      expect(method).toBeDisabled();
      await user.click(method);
    }
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(openExternal).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
    expect(useAuthStore.getState().user).toBeNull();
    expect(
      screen.getByRole("heading", { name: "Welcome back" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("heading", { name: "Continue with the NyxID app" }),
    ).not.toBeInTheDocument();

    await act(async () => {
      finishProfile(
        new Response(
          JSON.stringify(
            status === 200
              ? passwordAccount
              : {
                  error: "unavailable",
                  error_code: 1001,
                  message: "Profile unavailable",
                },
          ),
          { status },
        ),
      );
    });

    if (status === 200) {
      await waitFor(() =>
        expect(navigate).toHaveBeenCalledExactlyOnceWith({ to: "/assistant" }),
      );
      expect(useAuthStore.getState()).toMatchObject({
        user: passwordAccount,
        isAuthenticated: true,
        isLoading: false,
      });
    } else {
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Profile unavailable",
      );
      expect(navigate).not.toHaveBeenCalled();
      for (const method of [app, google, github, signUp])
        expect(method).toBeEnabled();
      await user.click(app);
      await screen.findByRole("heading", {
        name: "Continue with the NyxID app",
      });
      expect(fetch).toHaveBeenCalledWith(
        "/api/v1/auth/device/request",
        expect.anything(),
      );
    }
  },
);
