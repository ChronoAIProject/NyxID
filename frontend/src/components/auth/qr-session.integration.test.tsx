import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WebDeviceLogin } from "./web-device-login";
import { LoginPage } from "@/pages/login";
import { useConsentStore } from "@/stores/consent-store";
import { useAuthStore } from "@/stores/auth-store";

vi.mock("qrcode", () => ({
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,qr") },
}));
vi.mock("@/components/auth/auth-flow", () => ({
  AuthFlow: () => <div>All sign-in options</div>,
}));

const initial = useAuthStore.getState();
let completeProfile: (response: Response) => void;
const profile = {
  id: "qr-person",
  email: "qr@example.com",
  mfa_enabled: true,
  is_active: true,
  email_verified: true,
};
beforeEach(() => {
  localStorage.clear();
  useConsentStore.getState().bindAccount(null);
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (url) => {
      if (url === "/api/v1/auth/login")
        return new Response(
          JSON.stringify({
            error: "mfa_required",
            error_code: 2002,
            message: "MFA required",
            session_token: "pending-password-mfa",
          }),
          { status: 401 },
        );
      if (url === "/api/v1/auth/device/request")
        return new Response(
          JSON.stringify({
            device_code: "nyx_adc_private",
            user_code: "ABCD-EFGH",
            verification_uri: "https://id.example/login/device",
            verification_uri_complete:
              "https://id.example/login/device?user_code=ABCD-EFGH",
            expires_in: 600,
            interval: 1,
          }),
        );
      if (url === "/api/v1/auth/device/poll-web")
        return new Response(
          JSON.stringify({ ok: true, auth_kind: "account_session" }),
        );
      if (url === "/api/v1/users/me")
        return new Promise<Response>((resolve) => {
          completeProfile = resolve;
        });
      throw new Error(`Unexpected request: ${String(url)}`);
    }),
  );
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useAuthStore.setState(initial);
});

it("clears a pending password MFA challenge only after QR profile verification and navigates authenticated", async () => {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const root = createRootRoute();
  const qr = createRoute({
    path: "/qr",
    getParentRoute: () => root,
    component: WebDeviceLogin,
  });
  const dashboard = createRoute({
    path: "/assistant",
    getParentRoute: () => root,
    component: () => <div>Authenticated destination</div>,
    beforeLoad: () => {
      expect(useAuthStore.getState().isAuthenticated).toBe(true);
      expect(useAuthStore.getState().user?.mfa_enabled).toBe(true);
      expect(useAuthStore.getState().mfaRequired).toBe(false);
      expect(useAuthStore.getState().mfaToken).toBeNull();
    },
  });
  const router = createRouter({
    routeTree: root.addChildren([qr, dashboard]),
    history: createMemoryHistory({ initialEntries: ["/qr"] }),
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await userEvent.click(
    await screen.findByRole("button", { name: "Continue with the NyxID app" }),
  );
  await screen.findByText("ABCD-EFGH");
  await act(async () => {
    await useAuthStore.getState().login("qr@example.com", "password");
  });
  expect(useAuthStore.getState().mfaRequired).toBe(true);
  expect(useAuthStore.getState().mfaToken).toBe("pending-password-mfa");
  await waitFor(
    () =>
      expect(fetch).toHaveBeenCalledWith("/api/v1/users/me", expect.anything()),
    { timeout: 3000 },
  );
  expect(useAuthStore.getState().mfaRequired).toBe(true);
  await act(async () => {
    completeProfile(new Response(JSON.stringify(profile)));
  });
  await screen.findByText("Authenticated destination");
  expect(router.state.location.pathname).toBe("/assistant");
  cleanup();
  render(
    <QueryClientProvider client={client}>
      <LoginPage />
    </QueryClientProvider>,
  );
  expect(screen.getByText("All sign-in options")).toBeInTheDocument();
  expect(
    screen.queryByRole("heading", { name: "MFA verification" }),
  ).not.toBeInTheDocument();
});

it("still renders the TOTP gate for password authentication", async () => {
  await useAuthStore.getState().login("qr@example.com", "password");
  const root = createRootRoute({ component: LoginPage });
  const router = createRouter({
    routeTree: root,
    history: createMemoryHistory(),
  });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  expect(
    await screen.findByRole("heading", { name: "MFA verification" }),
  ).toBeInTheDocument();
  expect(screen.queryByText("All sign-in options")).not.toBeInTheDocument();
});

async function renderQr() {
  const root = createRootRoute();
  const qr = createRoute({
    path: "/qr",
    getParentRoute: () => root,
    component: WebDeviceLogin,
  });
  const dashboard = createRoute({
    path: "/assistant",
    getParentRoute: () => root,
    component: () => <div>Authenticated destination</div>,
  });
  const router = createRouter({
    routeTree: root.addChildren([qr, dashboard]),
    history: createMemoryHistory({ initialEntries: ["/qr"] }),
  });
  const view = render(
    <QueryClientProvider client={new QueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await userEvent.click(
    await screen.findByRole("button", { name: "Continue with the NyxID app" }),
  );
  await waitFor(
    () =>
      expect(fetch).toHaveBeenCalledWith("/api/v1/users/me", expect.anything()),
    { timeout: 3000 },
  );
  return { ...view, router };
}

const differentAccount = {
  ...profile,
  id: "different-person",
  email: "different@example.com",
  display_name: null,
  avatar_url: null,
  is_admin: false,
  created_at: "2026-01-01T00:00:00Z",
};
function profileFailure(status: number) {
  return new Response(
    JSON.stringify({
      error: status === 401 ? "unauthorized" : "unavailable",
      error_code: status === 401 ? 1001 : 1006,
      message: "Profile unavailable",
    }),
    { status },
  );
}
function callsTo(path: string) {
  return vi
    .mocked(fetch)
    .mock.calls.filter(([url]) => url === `/api/v1${path}`);
}

it.each([401, 503])(
  "requires fresh confirmation after profile %s and retries only the delivered session",
  async (status) => {
    useAuthStore.getState().setUser(differentAccount);
    useConsentStore.getState().setConsent(true);
    expect(useConsentStore.getState().accountId).toBe(differentAccount.id);
    await useAuthStore.getState().login("qr@example.com", "password");
    const { router } = await renderQr();
    expect(
      screen.getByText("Loading your approved account..."),
    ).toBeInTheDocument();
    expect(useAuthStore.getState()).toMatchObject({
      user: null,
      isAuthenticated: false,
      mfaRequired: true,
      mfaToken: "pending-password-mfa",
    });
    expect(useConsentStore.getState()).toMatchObject({
      accountId: null,
      enabled: false,
    });
    await act(async () => {
      completeProfile(profileFailure(status));
    });
    const retry = await screen.findByRole("button", {
      name: "Retry loading account",
    });
    expect(
      screen.getByText(
        /Your sign-in was approved, but we couldn't load your account/,
      ),
    ).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/qr");
    expect(
      screen.queryByText("Authenticated destination"),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByText("Signed in. Redirecting..."),
    ).not.toBeInTheDocument();
    expect(useAuthStore.getState().mfaRequired).toBe(true);
    expect(useAuthStore.getState().mfaToken).toBe("pending-password-mfa");
    expect(useAuthStore.getState().user).toBeNull();
    expect(useAuthStore.getState().isAuthenticated).toBe(false);
    await userEvent.click(retry);
    await waitFor(() => expect(callsTo("/users/me")).toHaveLength(2));
    expect(
      screen.getByText("Loading your approved account..."),
    ).toBeInTheDocument();
    expect(useAuthStore.getState()).toMatchObject({
      user: null,
      isAuthenticated: false,
      mfaRequired: true,
      mfaToken: "pending-password-mfa",
    });
    await act(async () => {
      completeProfile(new Response(JSON.stringify(profile)));
    });
    await screen.findByText("Authenticated destination");
    expect(useAuthStore.getState().user?.id).toBe(profile.id);
    expect(useConsentStore.getState()).toMatchObject({
      accountId: profile.id,
      enabled: false,
    });
    expect(useAuthStore.getState().isAuthenticated).toBe(true);
    expect(useAuthStore.getState().mfaRequired).toBe(false);
    expect(useAuthStore.getState().mfaToken).toBeNull();
    expect(callsTo("/auth/device/request")).toHaveLength(1);
    expect(callsTo("/auth/device/poll-web")).toHaveLength(1);
  },
);

it.each([
  ["restart", 200],
  ["restart", 401],
  ["restart", 503],
  ["unmount", 200],
  ["unmount", 401],
  ["unmount", 503],
] as const)(
  "fences a late profile %s/%s response from newer identity and MFA state",
  async (lifecycle, status) => {
    const view = await renderQr();
    const finishOldProfile = completeProfile;
    if (lifecycle === "unmount") view.unmount();
    else {
      await userEvent.click(
        screen.getByRole("button", { name: "Back to all sign-in options" }),
      );
      await userEvent.click(
        screen.getByRole("button", { name: "Continue with the NyxID app" }),
      );
      await screen.findByText("ABCD-EFGH");
    }
    act(() => {
      useAuthStore.getState().setUser(differentAccount);
      useAuthStore.getState().setMfaRequired(true, "newer-password-challenge");
      useAuthStore.setState({ isLoading: true });
    });
    const newerState = useAuthStore.getState();
    const newerConsent = useConsentStore.getState();
    await act(async () => {
      finishOldProfile(
        status === 200
          ? new Response(JSON.stringify(profile))
          : profileFailure(status),
      );
    });
    expect(useAuthStore.getState()).toBe(newerState);
    expect(useConsentStore.getState()).toBe(newerConsent);
    expect(view.router.state.location.pathname).toBe("/qr");
    expect(
      screen.queryByText("Authenticated destination"),
    ).not.toBeInTheDocument();
    if (lifecycle === "restart")
      expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
  },
);

it("fences profile application after cancellation while the response body is still loading", async () => {
  const view = await renderQr();
  let finishBody!: (value: unknown) => void;
  const response = new Response();
  vi.spyOn(response, "json").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishBody = resolve;
      }),
  );
  await act(async () => {
    completeProfile(response);
  });
  expect(response.json).toHaveBeenCalledOnce();
  view.unmount();
  act(() => {
    useAuthStore.getState().setUser(differentAccount);
    useAuthStore.getState().setMfaRequired(true, "newer-password-challenge");
  });
  const newerState = useAuthStore.getState();
  const newerConsent = useConsentStore.getState();
  await act(async () => {
    finishBody(profile);
  });
  expect(useAuthStore.getState()).toBe(newerState);
  expect(useConsentStore.getState()).toBe(newerConsent);
  expect(view.router.state.location.pathname).toBe("/qr");
});
