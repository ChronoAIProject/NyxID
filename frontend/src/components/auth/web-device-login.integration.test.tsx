import { StrictMode, useState, type FunctionComponent } from "react";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useLoginApproval } from "@/hooks/use-login-approval";
import { LoginPage } from "@/pages/login";
import { AuthFlow } from "./auth-flow";
import { LoginIdentityCard } from "./login-identity-card";
import { SignInStep } from "@/features/nyxbot-onboarding/sign-in-step";
import { WebDeviceLogin } from "./web-device-login";

const { post, approvalApi, checkAuth, authState, qrImage } = vi.hoisted(() => ({
  post: vi.fn(), approvalApi: vi.fn(), checkAuth: vi.fn(),
  authState: { mfaRequired: false },
  qrImage: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({
  api: { post }, apiClient: approvalApi, ApiError: class extends Error {},
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: Object.assign(
    (selector: (state: typeof authState & { checkAuth: typeof checkAuth }) => unknown) => selector({ ...authState, checkAuth }),
    { getState: () => ({ checkAuth }) },
  ),
}));
vi.mock("@/hooks/use-public-config", () => ({ usePublicConfig: () => ({
  data: { invite_code_required: false, email_auth_enabled: true, social_providers: [] },
  isSuccess: true, isPending: false, isError: false,
}) }));
vi.mock("@/hooks/use-auth", () => ({
  useLogin: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useRegister: () => ({ mutateAsync: vi.fn(), isPending: false }),
  useMfaVerify: () => ({ mutateAsync: vi.fn(), isPending: false }),
}));
vi.mock("@/hooks/use-theme", () => ({ useApplyTheme: () => {} }));
vi.mock("qrcode", () => ({ default: { toDataURL: qrImage } }));

const request = {
  device_code: "nyx_adc_private", user_code: "ABCD-EFGH",
  verification_uri: "https://id.example/login/device",
  verification_uri_complete: "https://id.example/login/device?user_code=ABCD-EFGH",
  app_verification_uri: "https://api.example/login/device?user_code=ABCD-EFGH",
  expires_in: 60, interval: 5,
};
const nextRequest = {
  ...request, device_code: "nyx_adc_next", user_code: "JKMN-PQRS",
  app_verification_uri: "https://api.example/login/device?user_code=JKMN-PQRS",
};
const identity = {
  id: "22c851d2-a23d-4a81-8f8e-804c3aa6f7c1", flow: "device", user_code: "WXYZ2345",
  verified: false, mfa_required: false, keep_signed_in: false,
  expires_at: "2026-08-20T10:10:00Z", user: null,
};
let coarse = true;

function deferred() {
  let resolve!: (value: unknown) => void;
  const promise = new Promise<unknown>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-08-20T10:00:00Z"));
  vi.clearAllMocks();
  sessionStorage.clear();
  authState.mfaRequired = false;
  coarse = true;
  const media = window.matchMedia("(pointer: coarse)");
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    ...media, media: query, matches: query === "(pointer: coarse)" && coarse,
    addEventListener: vi.fn(), removeEventListener: vi.fn(),
  }));
  vi.spyOn(window, "open").mockReturnValue(null);
  vi.spyOn(window.location, "assign").mockImplementation(() => undefined);
  post.mockReset().mockImplementation((path) => path === "/auth/device/request"
    ? Promise.resolve(request) : Promise.reject({ errorCode: 11202 }));
  approvalApi.mockReset();
  checkAuth.mockReset().mockResolvedValue(true);
  qrImage.mockReset().mockResolvedValue("data:image/png;base64,qr");
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  window.history.replaceState({}, "", "/");
  vi.clearAllTimers();
  vi.useRealTimers();
});

async function renderRoute(path = "/login", component: FunctionComponent = LoginPage) {
  window.history.replaceState({}, "", path);
  const root = createRootRoute();
  const route = createRoute({ getParentRoute: () => root, path: path.split("?")[0]!, component });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: [path] }) });
  const view = render(<StrictMode><QueryClientProvider client={new QueryClient()}><RouterProvider router={router} /></QueryClientProvider></StrictMode>);
  await act(async () => { await router.load(); });
  return { ...view, router };
}

async function click(name: string) {
  await act(async () => { fireEvent.click(screen.getByRole("button", { name })); });
}
async function openQrPanel() {
  await click("Continue with the NyxID app");
  expect(screen.getByText("ABCD-EFGH")).toBeVisible();
  expect(screen.getByAltText("QR code to continue with the NyxID app")).toBeVisible();
}
async function resumeAndPoll() {
  await act(async () => {
    window.dispatchEvent(new Event("pageshow"));
    document.dispatchEvent(new Event("visibilitychange"));
    await vi.advanceTimersByTimeAsync(5_000);
  });
}

it.each(["normal", "MFA", "cookie MFA"])("only the /login %s QR opts into confirmation, with no automatic opening", async (entry) => {
  authState.mfaRequired = entry === "MFA";
  const path = entry === "cookie MFA" ? "/login?mfa=required" : "/login";
  const { router } = await renderRoute(path);
  expect(post).not.toHaveBeenCalled();
  await openQrPanel();
  expect(qrImage).toHaveBeenCalledWith(request.app_verification_uri, expect.anything());
  expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await resumeAndPoll();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(window.open).not.toHaveBeenCalled();
  await click("Open NyxID app from QR code");
  expect(screen.getByRole("dialog", { name: "Open NyxID app?" })).toBeVisible();
  const link = screen.getByRole("link", { name: "Open NyxID app" });
  expect(link).toHaveAttribute("href", request.app_verification_uri);
  expect(link).toHaveAttribute("target", "_blank");
  expect(link).toHaveAttribute("rel", "noopener noreferrer");
  expect(link).toHaveAttribute("referrerpolicy", "no-referrer");
  expect(window.open).not.toHaveBeenCalled();
  expect(window.location.assign).not.toHaveBeenCalled();
  expect(router.state.location.pathname).toBe("/login");
  // Suppress browser navigation in the test; exercise the affirmative React click.
  link.addEventListener("click", (event) => event.preventDefault(), { once: true });
  fireEvent.click(link);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(router.state.location.pathname).toBe("/login");
});

it.each(["/login/device", "/login/agent-key", "/onboarding", "/default"])("%s uses only the web verification QR without handoff", async (path) => {
  const flow = path === "/login/agent-key" ? "agent-key" : "device";
  approvalApi.mockResolvedValueOnce({ ...identity, flow }).mockResolvedValueOnce(request);
  function RequestBoundIdentity() {
    const approval = useLoginApproval(flow, "WXYZ2345", "?user_code=WXYZ2345");
    return <LoginIdentityCard approval={approval} onVerified={() => {}} disabled={false} />;
  }
  const component = path === "/onboarding" ? () => <SignInStep returnTo="/assistant" />
    : path === "/default" ? () => <AuthFlow /> : RequestBoundIdentity;
  await renderRoute(path, component);
  const provider = screen.getAllByRole("button").find((button) => /NyxID app|NyxID 应用|appProvider/.test(button.textContent ?? ""));
  expect(provider).toBeDefined();
  await act(async () => { fireEvent.click(provider!); });
  expect(screen.getByText("ABCD-EFGH")).toBeVisible();
  const qr = screen.getByAltText("QR code to continue with the NyxID app");
  expect(qr.closest("button")).toBeNull();
  expect(qrImage).toHaveBeenCalledWith(request.verification_uri_complete, expect.anything());
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  fireEvent.click(qr);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(window.open).not.toHaveBeenCalled();
});

it.each([
  { name: "fine pointer", coarse: false, native: request.app_verification_uri },
  { name: "older backend", coarse: true, native: undefined },
  { name: "malformed URL", coarse: true, native: "not a URL" },
  { name: "executable URL", coarse: true, native: "javascript:alert(1)" },
  { name: "native scheme", coarse: true, native: "nyxid://login/device" },
  { name: "HTTP", coarse: true, native: "http://localhost:3001/login/device" },
])("/login retains a plain QR for $name", async (scenario) => {
  coarse = scenario.coarse;
  post.mockResolvedValueOnce({ ...request, app_verification_uri: scenario.native });
  await renderRoute();
  await openQrPanel();
  const qr = screen.getByAltText("QR code to continue with the NyxID app");
  expect(qr.closest("button")).toBeNull();
  expect(qrImage).toHaveBeenCalledWith(scenario.coarse ? request.verification_uri_complete : request.app_verification_uri, expect.anything());
  fireEvent.click(qr);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(window.open).not.toHaveBeenCalled();
});

it("Generate new never opens a dialog or app and stale confirmation never adopts the replacement", async () => {
  await renderRoute();
  await openQrPanel();
  await click("Open NyxID app from QR code");
  post.mockRejectedValueOnce({ errorCode: 11204 });
  await resumeAndPoll();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  post.mockResolvedValueOnce(nextRequest);
  await click("Generate new code");
  expect(screen.getByText("JKMN-PQRS")).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await click("Open NyxID app from QR code");
  expect(screen.getByRole("link", { name: "Open NyxID app" })).toHaveAttribute("href", nextRequest.app_verification_uri);
  expect(window.open).not.toHaveBeenCalled();
});

it.each(["QR tap", "confirmation"])("checks expiry at %s even before a suspended countdown runs", async (stage) => {
  await renderRoute();
  await openQrPanel();
  if (stage === "confirmation") await click("Open NyxID app from QR code");
  // Suspend timers and pass the original challenge deadline.
  vi.setSystemTime(new Date("2026-08-20T10:01:01Z"));
  if (stage === "QR tap") await click("Open NyxID app from QR code");
  else {
    const confirm = screen.getByRole("link", { name: "Open NyxID app" });
    const clickEvent = new MouseEvent("click", { bubbles: true, cancelable: true });
    act(() => { confirm.dispatchEvent(clickEvent); });
    expect(clickEvent.defaultPrevented).toBe(true);
  }
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  expect(window.open).not.toHaveBeenCalled();
});

it("Back then a throttled reopen cannot expose a stale response or open a dialog", async () => {
  const old = deferred();
  post.mockReturnValueOnce(old.promise).mockResolvedValueOnce(nextRequest);
  await renderRoute();
  await click("Continue with the NyxID app");
  await click("Back to all sign-in options");
  await click("Continue with the NyxID app");
  await act(async () => { old.resolve(request); });
  expect(post).toHaveBeenCalledTimes(1);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByText("ABCD-EFGH")).not.toBeInTheDocument();
  await click("Back to all sign-in options");
  await act(async () => { await vi.advanceTimersByTimeAsync(1_000); });
  await click("Continue with the NyxID app");
  expect(screen.getByText("JKMN-PQRS")).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(window.open).not.toHaveBeenCalled();
});

it("controlled close and reopen cannot revive a selected handoff", async () => {
  function ControlledLogin() {
    const [open, setOpen] = useState(false);
    return <>
      <button onClick={() => setOpen(false)}>Hide login</button>
      <button onClick={() => setOpen(true)}>Show login</button>
      <WebDeviceLogin isOpen={open} onOpenChange={setOpen} allowAppHandoff />
    </>;
  }
  await renderRoute("/login", ControlledLogin);
  await openQrPanel();
  await click("Open NyxID app from QR code");
  // Programmatic external close, while the modal owns the pointer/focus scope.
  await act(async () => { fireEvent.click(screen.getByRole("button", { name: "Hide login", hidden: true })); });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await click("Show login");
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Open NyxID app" })).not.toBeInTheDocument();
  await resumeAndPoll();
  expect(window.open).not.toHaveBeenCalled();
});

it("unmount discards confirmation and unresolved request state", async () => {
  const view = await renderRoute();
  await openQrPanel();
  await click("Open NyxID app from QR code");
  view.unmount();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await resumeAndPoll();
  expect(window.open).not.toHaveBeenCalled();
});

it("request-bound appTransport remains plain by default and still adopts approved proof", async () => {
  approvalApi.mockResolvedValueOnce(identity).mockResolvedValueOnce(request)
    .mockResolvedValueOnce({ ...identity, verified: true });
  const verified = vi.fn();
  function RequestBoundLogin() {
    const approval = useLoginApproval("device", "WXYZ2345", "?user_code=WXYZ2345");
    return <WebDeviceLogin transport={approval.appTransport(false)} onVerified={verified} />;
  }
  await renderRoute("/login/device", RequestBoundLogin);
  await openQrPanel();
  expect(qrImage).toHaveBeenCalledWith(request.verification_uri_complete, expect.anything());
  expect(screen.getByAltText("QR code to continue with the NyxID app").closest("button")).toBeNull();
  await resumeAndPoll();
  expect(verified).toHaveBeenCalledOnce();
  expect(post).not.toHaveBeenCalled();
  expect(checkAuth).not.toHaveBeenCalled();
  expect(window.open).not.toHaveBeenCalled();
});
