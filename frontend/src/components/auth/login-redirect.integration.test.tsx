import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi, type MockInstance } from "vitest";
import { CmaSessionProvider } from "@/components/assistant/cma-session-provider";
import { LoginPage } from "@/pages/login";
import { RegisterPage } from "@/pages/register";
import { useAuthStore } from "@/stores/auth-store";
import { useLoginApproval } from "@/hooks/use-login-approval";
import { LoginIdentityCard } from "./login-identity-card";

vi.mock("qrcode", () => ({
  default: { toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,qr") },
}));

const initial = useAuthStore.getState();
const conversation = "nyxa-4c9f28dd584b44a79bb4f452528d814a";
const returnPath = `/assistant?c=${conversation}`;
const profile = {
  id: "app-person",
  email: "app@example.com",
  display_name: "App person",
  avatar_url: null,
  mfa_enabled: true,
  is_active: true,
  email_verified: true,
  is_admin: false,
  created_at: "2026-01-01T00:00:00Z",
};
const approvalId = "11111111-1111-4111-8111-111111111111";
let passwordMfa: boolean;
let finishProfile: (response: Response) => void;
let client: QueryClient;
let assign: MockInstance<typeof window.location.assign>;
const verified = vi.fn();

function callsTo(endpoint: string) {
  return vi
    .mocked(fetch)
    .mock.calls.filter(([url]) => url === `/api/v1${endpoint}`);
}

beforeEach(() => {
  passwordMfa = false;
  localStorage.clear();
  sessionStorage.clear();
  verified.mockClear();
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    isLoading: false,
    mfaRequired: false,
    mfaToken: null,
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  assign = vi
    .spyOn(window.location, "assign")
    .mockImplementation(() => undefined);
  let keepSignedIn = false;
  const identity = () => ({
    id: approvalId,
    flow: "device",
    user_code: "JKLMNPQR",
    keep_signed_in: keepSignedIn,
    verified: false,
    mfa_required: false,
    expires_at: new Date(Date.now() + 600_000).toISOString(),
    user: null,
  });
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>(async (url, options) => {
      const endpoint = String(url).replace("/api/v1", "");
      if (endpoint === "/public/config")
        return Response.json({
          social_providers: ["google"],
          email_auth_enabled: true,
          invite_code_required: false,
        });
      if (endpoint === "/auth/login" && passwordMfa)
        return Response.json(
          {
            error: "mfa_required",
            error_code: 2002,
            message: "MFA required",
            session_token: "pending-password-mfa",
          },
          { status: 401 },
        );
      if (["/auth/login", "/auth/mfa/verify"].includes(endpoint))
        return Response.json({});
      if (endpoint === "/auth/register")
        return Response.json({ message: "Check your email" });
      if (endpoint === "/users/me")
        return new Promise<Response>((resolve) => {
          finishProfile = resolve;
        });
      if (
        endpoint === "/auth/device/request" ||
        endpoint === `/auth/approval/${approvalId}/app`
      )
        return Response.json({
          device_code: "nyx_adc_private",
          user_code: "ABCD-EFGH",
          verification_uri: `${window.location.origin}/login/device`,
          verification_uri_complete: `${window.location.origin}/login/device?user_code=ABCD-EFGH`,
          expires_in: 600,
          interval: 1,
        });
      if (endpoint === "/auth/device/poll-web")
        return Response.json({ ok: true, auth_kind: "account_session" });
      if (endpoint === "/auth/approval") {
        keepSignedIn = JSON.parse(String(options?.body)).keep_signed_in;
        return Response.json(identity());
      }
      if (endpoint === `/auth/approval/${approvalId}/app/poll`)
        return Response.json({
          ...identity(),
          verified: true,
          user: {
            id: profile.id,
            email: profile.email,
            display_name: profile.display_name,
          },
        });
      throw Error(`Unexpected request: ${endpoint}`);
    }),
  );
});

afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
  assign.mockRestore();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  useAuthStore.setState(initial);
  window.history.replaceState({}, "", "/");
});

function ApprovalPage() {
  const approval = useLoginApproval(
    "device",
    "JKLMNPQR",
    "?user_code=JKLM-NPQR",
  );
  return (
    <LoginIdentityCard
      approval={approval}
      onVerified={verified}
      disabled={false}
    />
  );
}

async function open(
  path = "/login",
  returnTo: string | undefined = returnPath,
  cookieMfa = false,
) {
  const params = new URLSearchParams();
  if (returnTo) params.set("return_to", returnTo);
  if (cookieMfa) params.set("mfa", "required");
  const entry = `${path}?${params}`;
  window.history.replaceState({}, "", entry);
  const root = createRootRoute({
    component: () => (
      <CmaSessionProvider>
        <Outlet />
      </CmaSessionProvider>
    ),
  });
  const login = createRoute({
    path: "/login",
    getParentRoute: () => root,
    component: LoginPage,
  });
  const register = createRoute({
    path: "/register",
    getParentRoute: () => root,
    component: RegisterPage,
  });
  const approval = createRoute({
    path: "/approval",
    getParentRoute: () => root,
    component: ApprovalPage,
  });
  const assistant = createRoute({
    path: "/assistant",
    getParentRoute: () => root,
    component: () => <div>Assistant destination</div>,
  });
  const router = createRouter({
    routeTree: root.addChildren([login, register, approval, assistant]),
    history: createMemoryHistory({ initialEntries: [entry] }),
  });
  render(
    <QueryClientProvider client={client}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
  await act(() => vi.dynamicImportSettled());
  return router;
}

async function passwordLogin() {
  await userEvent.type(
    await screen.findByPlaceholderText("you@example.com"),
    profile.email,
  );
  await userEvent.type(
    screen.getByPlaceholderText("Enter your password"),
    "correct horse",
  );
  await userEvent.click(screen.getByRole("button", { name: "Sign in" }));
}

async function confirmProfile() {
  await waitFor(() => expect(callsTo("/users/me")).toHaveLength(1), {
    timeout: 3000,
  });
  expect(assign).not.toHaveBeenCalled();
  await act(async () => {
    finishProfile(Response.json(profile));
  });
}

async function startAppLogin() {
  const trigger = await screen.findByRole("button", {
    name: "Continue with the NyxID app",
  });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    await act(async () => {
      fireEvent.click(trigger);
    });
    expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
  } finally {
    vi.useRealTimers();
  }
}

it.each(["methods", "password MFA", "social MFA", "phone resume"])(
  "preserves the assistant conversation after app login from %s through the global provider",
  async (start) => {
    const target = `${window.location.origin}${returnPath}`;
    await open("/login", target, start === "social MFA");
    if (start === "password MFA") {
      passwordMfa = true;
      await passwordLogin();
      await screen.findByRole("heading", { name: "MFA verification" });
    }
    await startAppLogin();
    if (start === "phone resume") {
      // Model returning to the requesting tab after approving in the app.
      act(() => {
        window.dispatchEvent(
          new PageTransitionEvent("pageshow", { persisted: true }),
        );
        document.dispatchEvent(new Event("visibilitychange"));
      });
      expect(window.location.search).toContain("return_to=");
    }
    await confirmProfile();
    await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(target));
    expect(callsTo("/auth/device/request")).toHaveLength(1);
    expect(callsTo("/auth/device/poll-web")).toHaveLength(1);
    expect(callsTo("/auth/mfa/verify")).toHaveLength(0);
    expect(useAuthStore.getState()).toMatchObject({
      isAuthenticated: true,
      mfaRequired: false,
    });
    expect(new URL(assign.mock.calls[0]![0]).searchParams.get("c")).toBe(
      conversation,
    );
  },
);

it.each([
  undefined,
  "//evil.example/assistant",
  "https://evil.example/assistant",
  "javascript:alert(1)",
])(
  "uses the Assistant fallback for app login with return_to=%s",
  async (returnTo) => {
    const router = await open("/login", returnTo ?? "");
    await startAppLogin();
    await confirmProfile();
    await screen.findByText("Assistant destination");
    expect(router.state.location.pathname).toBe("/assistant");
    expect(assign).not.toHaveBeenCalled();
  },
);

it("retains a configured backend return URL after app session restoration", async () => {
  vi.stubEnv("VITE_BACKEND_URL", "https://api.example.test/api/v1");
  const target =
    "https://api.example.test/oauth/authorize?client_id=app&state=resume";
  await open("/login", target);
  await startAppLogin();
  await confirmProfile();
  await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(target));
});

it("preserves a relative assistant return URL after password login", async () => {
  await open();
  await screen.findByRole("button", { name: "Continue with Google" });
  await passwordLogin();
  await confirmProfile();
  await waitFor(() =>
    expect(assign).toHaveBeenCalledExactlyOnceWith(
      `${window.location.origin}${returnPath}`,
    ),
  );
});

it.each([
  { cookie: false, recovery: false },
  { cookie: false, recovery: true },
  { cookie: true, recovery: false },
  { cookie: true, recovery: true },
])(
  "returns to the assistant after MFA (social cookie: $cookie, recovery: $recovery)",
  async ({ cookie, recovery }) => {
    const target = `${window.location.origin}${returnPath}`;
    await open("/login", target, cookie);
    if (!cookie) {
      passwordMfa = true;
      await screen.findByRole("button", { name: "Continue with Google" });
      await passwordLogin();
    }
    await screen.findByRole("heading", { name: "MFA verification" });
    if (recovery)
      await userEvent.click(
        screen.getByRole("button", { name: "Use a recovery code" }),
      );
    const code = recovery ? "9b7130d2e5a6417780c9fd2e54b602ad" : "123456";
    await userEvent.type(
      screen.getByLabelText(recovery ? "Recovery code" : "Authenticator code"),
      code,
    );
    await userEvent.click(screen.getByRole("button", { name: "Verify" }));
    await confirmProfile();
    await waitFor(() => expect(assign).toHaveBeenCalledExactlyOnceWith(target));
    const body = JSON.parse(String(callsTo("/auth/mfa/verify")[0]![1]?.body));
    expect(body).toEqual({
      code,
      client: "web",
      ...(!cookie && { mfa_token: "pending-password-mfa" }),
    });
  },
);

it.each(["/login", "/register"])(
  "passes the complete return URL to social login from %s",
  async (path) => {
    await open(path);
    await userEvent.click(
      await screen.findByRole("button", { name: "Continue with Google" }),
    );
    await waitFor(() => expect(assign).toHaveBeenCalledOnce());
    const target = new URL(assign.mock.calls[0]![0]);
    expect(target.pathname).toBe("/api/v1/auth/social/google");
    expect(target.searchParams.get("return_to")).toBe(returnPath);
    expect(target.searchParams.has("client")).toBe(false);
  },
);

it("keeps the conversation return URL through email registration and subsequent login", async () => {
  await open("/register");
  const email = await screen.findByRole("button", { name: "Continue with Email" });
  vi.useFakeTimers();
  await act(async () => {
    fireEvent.click(email);
  });
  // Finish the delayed focus before typing, so it cannot steal another field.
  await act(async () => {
    await vi.advanceTimersByTimeAsync(350);
  });
  expect(screen.getByLabelText("Full Name")).toHaveFocus();
  vi.useRealTimers();
  await userEvent.type(
    screen.getByLabelText("Full Name"),
    profile.display_name,
  );
  await userEvent.type(screen.getByLabelText("Email"), profile.email);
  await userEvent.type(
    screen.getByLabelText("Password", { exact: true }),
    "CorrectHorse123!",
  );
  await userEvent.type(
    screen.getByLabelText("Confirm Password"),
    "CorrectHorse123!",
  );
  vi.useFakeTimers();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Create Account" }));
  });
  expect(callsTo("/auth/register")).toHaveLength(1);
  expect(JSON.parse(String(callsTo("/auth/register")[0]![1]?.body))).toEqual({
    display_name: profile.display_name,
    email: profile.email,
    password: "CorrectHorse123!",
    invite_code: "",
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(200);
  });
  expect(screen.getByRole("heading", { name: "Welcome back" })).toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(250);
  });
  expect(screen.getByRole("heading", { name: "Welcome back" })).toBeVisible();
  expect(screen.getByPlaceholderText("you@example.com")).toHaveFocus();
  expect(window.location.pathname).toBe("/login");
  expect(new URLSearchParams(window.location.search).get("return_to")).toBe(
    returnPath,
  );
  vi.useRealTimers();
  await passwordLogin();
  await confirmProfile();
  await waitFor(() =>
    expect(assign).toHaveBeenCalledExactlyOnceWith(
      `${window.location.origin}${returnPath}`,
    ),
  );
});

it.each([false, true])(
  "completes request-bound app verification without navigating (keep signed in: %s)",
  async (keep) => {
    await open("/approval", "");
    if (keep)
      await userEvent.click(
        await screen.findByRole("button", { name: /Keep me signed in/ }),
      );
    await startAppLogin();
    if (keep) await confirmProfile();
    await waitFor(() => expect(verified).toHaveBeenCalledOnce(), {
      timeout: 3000,
    });
    expect(assign).not.toHaveBeenCalled();
    expect(useAuthStore.getState().isAuthenticated).toBe(keep);
    expect(callsTo("/users/me")).toHaveLength(keep ? 1 : 0);
    expect(callsTo("/auth/device/poll-web")).toHaveLength(0);
    expect(callsTo(`/auth/approval/${approvalId}/app/poll`)).toHaveLength(1);
    expect(callsTo(`/auth/approval/${approvalId}/approve`)).toHaveLength(0);
    expect(window.location.pathname).toBe("/approval");
  },
);
