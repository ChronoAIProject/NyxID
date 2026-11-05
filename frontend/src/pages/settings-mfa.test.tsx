import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
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
import { TooltipProvider } from "@/components/ui/tooltip";
import { validateSettingsSearch } from "@/lib/assistant/account-panel-search";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { SettingsPage } from "./settings";
import { useUser } from "@/hooks/use-auth";

function AuthenticatedSettings() {
  useUser();
  return <SettingsPage />;
}

vi.mock("@/hooks/use-public-config", () => ({
  usePublicConfig: () => ({ data: { email_auth_enabled: true } }),
}));
vi.mock("qrcode", () => ({
  default: { toDataURL: async () => "data:image/png;base64,fixture" },
}));
const initialAuth = useAuthStore.getState();
const transport = vi.fn<typeof fetch>();
let serverUser: User;
let client: QueryClient;
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
const setup = {
  secret: "fixture-secret",
  qr_code_url: "otpauth://totp/NyxID?secret=fixture",
};
beforeEach(() => {
  serverUser = {
    id: "owner",
    email: "reader@example.com",
    display_name: "Reader",
    mfa_enabled: false,
    has_password: true,
  } as User;
  useAuthStore.setState({ user: serverUser });
  transport.mockReset().mockImplementation(async (url, options) => {
    if (url === "/api/v1/users/me") return json(serverUser);
    if (url === "/api/v1/onboarding/bindings") return json([]);
    if (url === "/api/v1/auth/mfa/setup") return json(setup);
    if (url === "/api/v1/auth/mfa/confirm") {
      serverUser = { ...serverUser, mfa_enabled: true };
      return json({ recovery_codes: ["recovery-one", "recovery-two"] });
    }
    if (url === "/api/v1/auth/mfa/disable") {
      serverUser = { ...serverUser, mfa_enabled: false };
      return json({});
    }
    throw new Error(`Unexpected request: ${options?.method} ${String(url)}`);
  });
  vi.stubGlobal("fetch", transport);
});
afterEach(() => {
  cleanup();
  client?.clear();
  vi.unstubAllGlobals();
  useAuthStore.setState(initialAuth);
});
async function open(enabled = false) {
  serverUser = { ...serverUser, mfa_enabled: enabled };
  const root = createRootRoute();
  const route = createRoute({
    path: "/settings",
    getParentRoute: () => root,
    validateSearch: validateSettingsSearch,
    component: AuthenticatedSettings,
  });
  const router = createRouter({
    routeTree: root.addChildren([route]),
    history: createMemoryHistory({
      initialEntries: ["/settings?tab=security"],
    }),
  });
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <RouterProvider router={router} />
      </TooltipProvider>
    </QueryClientProvider>,
  );
  await act(() => router.load());
  const toggle = await screen.findByRole("switch", {
    name: "Toggle two-factor authentication",
  });
  await waitFor(() =>
    expect(toggle).toHaveAttribute("aria-checked", String(enabled)),
  );
  await screen.findByText(/No active onboarding bindings/);
  return toggle;
}
async function beginVerification(toggle: HTMLElement) {
  await userEvent.click(toggle);
  await userEvent.click(screen.getByRole("button", { name: "Begin Setup" }));
  return screen.findByRole("dialog", { name: "Scan QR Code" });
}

it("opens the real setup flow and cancelling either setup step leaves the switch off", async () => {
  const toggle = await open();
  await userEvent.click(toggle);
  const start = screen.getByRole("dialog", {
    name: "Enable Two-Factor Authentication",
  });
  expect(toggle).toHaveAttribute("aria-checked", "false");
  await userEvent.click(within(start).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(toggle).toHaveAttribute("aria-checked", "false");
  const verify = await beginVerification(toggle);
  expect(transport).toHaveBeenCalledWith(
    "/api/v1/auth/mfa/setup",
    expect.objectContaining({ method: "POST" }),
  );
  expect(toggle).toHaveAttribute("aria-checked", "false");
  await userEvent.type(within(verify).getByRole("textbox"), "123456");
  await userEvent.click(within(verify).getByRole("button", { name: "Cancel" }));
  expect(toggle).toHaveAttribute("aria-checked", "false");
  expect(
    transport.mock.calls.some(([url]) => url === "/api/v1/auth/mfa/confirm"),
  ).toBe(false);
});

it("announces setup pending/errors and prevents dismissal during preparation", async () => {
  let finish!: (response: Response) => void;
  const original = transport.getMockImplementation()!;
  transport.mockImplementation((url, options) =>
    url === "/api/v1/auth/mfa/setup"
      ? new Promise<Response>((resolve) => {
          finish = resolve;
        })
      : original(url, options),
  );
  const toggle = await open();
  await userEvent.click(toggle);
  await userEvent.click(screen.getByRole("button", { name: "Begin Setup" }));
  expect(screen.getByRole("status")).toHaveTextContent(
    "Preparing two-factor authentication",
  );
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await userEvent.keyboard("{Escape}");
  expect(
    screen.getByRole("button", { name: "Begin Setup" }).parentElement,
  ).toHaveAttribute("aria-busy", "true");
  expect(toggle).toHaveAttribute("aria-checked", "false");
  await act(async () =>
    finish(
      json(
        {
          error: "validation_error",
          error_code: 1000,
          message: "Setup unavailable",
        },
        400,
      ),
    ),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Setup unavailable",
  );
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(toggle).toHaveAttribute("aria-checked", "false");
});

it("announces verification pending/errors and changes the switch only after server confirmation", async () => {
  const toggle = await open();
  const dialog = await beginVerification(toggle);
  let finish!: (response: Response) => void;
  const original = transport.getMockImplementation()!;
  transport.mockImplementation((url, options) =>
    url === "/api/v1/auth/mfa/confirm"
      ? new Promise<Response>((resolve) => {
          finish = resolve;
        })
      : original(url, options),
  );
  await userEvent.type(within(dialog).getByRole("textbox"), "123456");
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Verify and Enable" }),
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Verifying authentication code",
  );
  expect(toggle).toHaveAttribute("aria-checked", "false");
  expect(within(dialog).getByRole("button", { name: "Cancel" })).toBeDisabled();
  await userEvent.keyboard("{Escape}");
  expect(dialog).toBeInTheDocument();
  await act(async () =>
    finish(
      json(
        {
          error: "validation_error",
          error_code: 1000,
          message: "Code rejected",
        },
        400,
      ),
    ),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Code rejected");
  expect(toggle).toHaveAttribute("aria-checked", "false");
  transport.mockImplementation(original);
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Verify and Enable" }),
  );
  await screen.findByRole("dialog", { name: "Recovery Codes" });
  await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
  expect(transport).toHaveBeenCalledWith(
    "/api/v1/auth/mfa/confirm",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ code: "123456" }),
    }),
  );
  expect(useAuthStore.getState().user?.mfa_enabled).toBe(true);
  await userEvent.click(screen.getByRole("button", { name: "Done" }));
});

it("keeps the enabled server state on cancel and error, announces disabling and refreshes only after success", async () => {
  const toggle = await open(true);
  await userEvent.click(toggle);
  let dialog = screen.getByRole("dialog", {
    name: "Disable Two-Factor Authentication",
  });
  expect(toggle).toHaveAttribute("aria-checked", "true");
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(toggle).toHaveAttribute("aria-checked", "true");
  await userEvent.click(toggle);
  dialog = screen.getByRole("dialog", {
    name: "Disable Two-Factor Authentication",
  });
  await userEvent.type(
    within(dialog).getByLabelText("Authenticator or recovery code"),
    "123456",
  );
  await userEvent.type(
    within(dialog).getByLabelText("Password"),
    "current-password",
  );
  await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await userEvent.click(screen.getByRole("button", { name: "Discard" }));
  expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(
    transport.mock.calls.some(([url]) => url === "/api/v1/auth/mfa/disable"),
  ).toBe(false);
  await userEvent.click(toggle);
  dialog = screen.getByRole("dialog", {
    name: "Disable Two-Factor Authentication",
  });
  await userEvent.type(
    within(dialog).getByLabelText("Authenticator or recovery code"),
    "123456",
  );
  await userEvent.type(
    within(dialog).getByLabelText("Password"),
    "current-password",
  );
  let finish!: (response: Response) => void;
  const original = transport.getMockImplementation()!;
  transport.mockImplementation((url, options) =>
    url === "/api/v1/auth/mfa/disable"
      ? new Promise<Response>((resolve) => {
          finish = resolve;
        })
      : original(url, options),
  );
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Disable MFA" }),
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "Disabling two-factor authentication",
  );
  expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(toggle).toBeDisabled();
  await userEvent.keyboard("{Escape}");
  expect(dialog).toBeInTheDocument();
  await act(async () =>
    finish(
      json(
        {
          error: "validation_error",
          error_code: 1000,
          message: "Code rejected",
        },
        400,
      ),
    ),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Code rejected");
  expect(toggle).toHaveAttribute("aria-checked", "true");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  transport.mockImplementation(original);
  await userEvent.click(
    within(dialog).getByRole("button", { name: "Disable MFA" }),
  );
  await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
  expect(transport).toHaveBeenCalledWith(
    "/api/v1/auth/mfa/disable",
    expect.objectContaining({
      method: "POST",
      body: JSON.stringify({ code: "123456", password: "current-password" }),
    }),
  );
  expect(useAuthStore.getState().user?.mfa_enabled).toBe(false);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
