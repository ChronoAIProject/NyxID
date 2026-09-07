import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LoginAgentKeyPage } from "./login-agent-key";
import { LoginCredentialsSection } from "@/components/dashboard/api-key-detail/login-credentials-section";
import { agentKeySummarySchema } from "@/schemas/agent-key-login";

const mocks = vi.hoisted(() => ({
  post: vi.fn(),
  get: vi.fn(),
  remove: vi.fn(),
  preview: vi.fn(),
  logout: vi.fn(),
  auth: { signedIn: true },
}));
vi.mock("@/lib/api-client", () => ({
  api: { post: mocks.post, get: mocks.get, delete: mocks.remove },
  apiClient: mocks.preview,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({
    user: { id: "user", display_name: "Human" },
    isAuthenticated: mocks.auth.signedIn,
    logout: mocks.logout,
  }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    search,
  }: {
    children: React.ReactNode;
    to: string;
    search?: { return_to?: string };
  }) => (
    <a
      href={`${to}${search?.return_to ? `?return_to=${encodeURIComponent(search.return_to)}` : ""}`}
    >
      {children}
    </a>
  ),
  useNavigate: () => vi.fn(),
}));
vi.mock("qrcode", () => ({
  default: {
    toDataURL: vi.fn().mockResolvedValue("data:image/png;base64,AA=="),
  },
}));

const key = agentKeySummarySchema.parse({
  id: "key",
  name: "Shared key",
  key_prefix: "nyxid_ag_12345678",
  owner_type: "personal",
  owner_id: "user",
  owner_name: "Human",
  scopes: "read proxy",
  allow_all_services: false,
  allow_all_nodes: false,
  allowed_service_ids: ["svc"],
  allowed_node_ids: [],
  allowed_services: [{ id: "svc", name: "Allowed service", owner_id: "user" }],
  allowed_nodes: [],
  expires_at: null,
  rate_limit_per_second: 10,
  rate_limit_burst: 20,
  platform: "generic",
  created_now: false,
});
const options = {
  keys: [key],
  services: key.allowed_services,
  nodes: [],
  orgs: [],
};
const preview = {
  client_label: "workstation",
  client_ip: "203.0.113.5",
  client_ip_attribution: "verified",
  requested_profile: "home-agent",
  status: "pending",
  initiated_at: "2026-01-01T00:00:00Z",
  expires_at: "2026-01-01T00:10:00Z",
  seconds_remaining: 600,
  interval: 5,
  api_key: null,
};
let client: QueryClient;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  vi.clearAllMocks();
  mocks.auth.signedIn = true;
  mocks.preview.mockResolvedValue(preview);
  mocks.post.mockImplementation(async (path: string) =>
    path.endsWith("options") ? options : { ok: true },
  );
  mocks.logout.mockResolvedValue(undefined);
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});
function mount(element: React.ReactNode = <LoginAgentKeyPage />) {
  render(<QueryClientProvider client={client}>{element}</QueryClientProvider>);
}
async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(800);
  });
}
async function review() {
  fireEvent.change(screen.getByLabelText("User code"), {
    target: { value: "ABCD-EFGH" },
  });
  await click("Continue");
}

describe("Agent Key login page and hooks", () => {
  it("makes no request on mount, focus or typing; preview is anonymous and explicit", async () => {
    mount();
    fireEvent.focus(screen.getByLabelText("User code"));
    fireEvent.change(screen.getByLabelText("User code"), {
      target: { value: "ABCD-EFGH" },
    });
    expect(mocks.preview).not.toHaveBeenCalled();
    expect(mocks.post).not.toHaveBeenCalled();
    await click("Continue");
    expect(mocks.preview).toHaveBeenCalledWith(
      "/auth/agent-key/preview",
      expect.objectContaining({
        credentials: "omit",
        body: { user_code: "ABCDEFGH" },
      }),
    );
    expect(screen.getByText("home-agent")).toBeInTheDocument();
    expect(screen.getByText("203.0.113.5")).toBeInTheDocument();
  });
  it("selects an existing key, confirms issuance and approves once", async () => {
    mount();
    await review();
    await click("Approve on this computer");
    fireEvent.click(screen.getByRole("radio", { name: "Shared key" }));
    await click("Review permissions");
    expect(
      screen.getByText(/existing secret and other consumers are unaffected/),
    ).toBeInTheDocument();
    expect(screen.getByText("Allowed service")).toBeInTheDocument();
    expect(screen.getByText("10 requests/s; burst 20")).toBeInTheDocument();
    await click("Approve");
    expect(mocks.post).toHaveBeenCalledWith("/auth/agent-key/approve", {
      user_code: "ABCDEFGH",
      selection: { kind: "existing", api_key_id: "key" },
    });
    expect(
      screen.getByText("Approved - return to your terminal"),
    ).toBeInTheDocument();
    expect(screen.queryByText("home-agent")).not.toBeInTheDocument();
    await click("Sign out of this browser");
    expect(mocks.logout).toHaveBeenCalledOnce();
  });
  it("creates a limited key with an explicit ninety-day expiry and confirms it", async () => {
    mount();
    await review();
    await click("Approve on this computer");
    fireEvent.click(screen.getByRole("radio", { name: "Create a new key" }));
    await click("Review permissions");
    expect(
      screen.getByText("Confirm effective permissions"),
    ).toBeInTheDocument();
    await click("Approve");
    expect(mocks.post).toHaveBeenCalledWith(
      "/auth/agent-key/approve",
      expect.objectContaining({
        selection: expect.objectContaining({
          kind: "new",
          scopes: "read proxy",
          allow_all_services: false,
          allow_all_nodes: false,
          allowed_service_ids: [],
          allowed_node_ids: [],
          expires_at: expect.any(String),
        }),
      }),
    );
  });
  it("rejects explicitly without issuing a credential", async () => {
    mount();
    await review();
    await click("Reject");
    expect(mocks.post).toHaveBeenCalledWith("/auth/agent-key/deny", {
      user_code: "ABCDEFGH",
    });
    expect(screen.getByText("Login rejected")).toBeInTheDocument();
  });
  it.each(["approved", "delivered", "denied", "expired"])(
    "phone polling stops at %s and clears temporary state without storage writes",
    async (status) => {
      mocks.auth.signedIn = false;
      const storage = vi.spyOn(Storage.prototype, "setItem");
      mount();
      await review();
      expect(
        screen.getByRole("link", { name: "Approve on this computer" }),
      ).toHaveAttribute("href", "/login?return_to=%2Flogin%2Fagent-key");
      await click("Approve from your phone");
      expect(
        screen.getByAltText("Agent Key login QR code"),
      ).toBeInTheDocument();
      mocks.preview.mockResolvedValue({ ...preview, status });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000);
      });
      const calls = mocks.preview.mock.calls.length;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(30000);
      });
      expect(mocks.preview).toHaveBeenCalledTimes(calls);
      expect(
        screen.queryByAltText("Agent Key login QR code"),
      ).not.toBeInTheDocument();
      expect(mocks.post).not.toHaveBeenCalled();
      expect(storage).not.toHaveBeenCalled();
      storage.mockRestore();
    },
  );
  it("displays failed previews and expiry without enabling approval", async () => {
    mocks.preview.mockRejectedValueOnce(new Error("Unavailable"));
    mount();
    await review();
    expect(screen.getByText("Unavailable")).toBeInTheDocument();
    mocks.preview.mockResolvedValueOnce({ ...preview, status: "expired" });
    await click("Continue");
    expect(screen.getByText("Login request expired")).toBeInTheDocument();
  });
  it("clears review context and grants when its deadline expires", async () => {
    mount();
    await review();
    await click("Approve on this computer");
    fireEvent.click(screen.getByRole("radio", { name: "Shared key" }));
    await click("Review permissions");
    await act(async () => {
      await vi.advanceTimersByTimeAsync(600000);
    });
    expect(screen.getByText("Login request expired")).toBeInTheDocument();
    expect(screen.queryByText("home-agent")).not.toBeInTheDocument();
    expect(screen.queryByText("Shared key")).not.toBeInTheDocument();
    expect(
      mocks.post.mock.calls.some(([path]) => path.endsWith("approve")),
    ).toBe(false);
  });
  it("lists login credentials and requires confirmation before revocation", async () => {
    mocks.get.mockResolvedValue({
      credentials: [
        {
          id: "cred",
          label: "workstation",
          secret_prefix: "nyxid_ag_12345678",
          is_active: true,
          revoked_reason: null,
          revoked_at: null,
          expires_at: null,
          last_used_at: null,
          created_at: "2026-01-01T00:00:00Z",
        },
      ],
    });
    mocks.remove.mockResolvedValue({ ok: true });
    mount(<LoginCredentialsSection keyId="key" />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(screen.getByText("workstation")).toBeInTheDocument();
    await click("Revoke");
    expect(mocks.remove).not.toHaveBeenCalled();
    await act(async () => {
      fireEvent.click(
        screen.getByRole("dialog").querySelector("button.bg-destructive") ??
          screen.getAllByRole("button", { name: "Revoke" }).at(-1)!,
      );
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mocks.remove).toHaveBeenCalledWith("/api-keys/key/credentials/cred");
  });
});
