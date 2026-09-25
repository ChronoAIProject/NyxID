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
  search: {} as { user_code?: string },
  navigate: vi.fn(),
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
  useNavigate: () => mocks.navigate,
  useSearch: () => mocks.search,
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
};
let client: QueryClient;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  vi.clearAllMocks();
  mocks.auth.signedIn = true;
  mocks.search = {};
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
  return render(
    <QueryClientProvider client={client}>{element}</QueryClientProvider>,
  );
}
async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(800);
  });
}
describe("Login code minting and credential management", () => {
  function mintResponses() {
    mocks.post.mockImplementation(async (path: string) =>
      path.endsWith("options")
        ? options
        : {
            request_id: "mint",
            code: "ABCD-EFGH",
            expires_at: "2026-01-01T00:05:00Z",
          },
    );
    mocks.get.mockResolvedValue({
      request_id: "mint",
      status: "pending",
      auth_kind: "agent_key",
      expires_at: "2026-01-01T00:05:00Z",
      redeemed_at: null,
      client_label: null,
      client_user_agent: null,
      client_ip: null,
      client_ip_attribution: "unavailable",
      can_revoke: false,
    });
  }
  it("mints an account code only after confirmation and clears its value on expiry", async () => {
    mintResponses();
    mount(<LoginAgentKeyPage mint />);
    await click("Full account session");
    expect(mocks.post).not.toHaveBeenCalled();
    await click("Generate account login code");
    expect(mocks.post).toHaveBeenCalledExactlyOnceWith("/auth/login-code", {
      auth_kind: "account_session",
    });
    expect(screen.getByText("ABCD-EFGH")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300_000);
    });
    expect(screen.queryByText("ABCD-EFGH")).not.toBeInTheDocument();
    expect(screen.getByText("Login code closed")).toBeInTheDocument();
  });
  it("mints an existing key only after reviewing its permissions", async () => {
    mintResponses();
    mount(<LoginAgentKeyPage mint />);
    await click("Restricted Agent Key");
    fireEvent.click(screen.getByRole("radio", { name: "Shared key" }));
    await click("Review permissions");
    expect(mocks.post).not.toHaveBeenCalledWith(
      "/auth/login-code",
      expect.anything(),
    );
    await click("Generate restricted login code");
    expect(mocks.post).toHaveBeenCalledWith("/auth/login-code", {
      auth_kind: "agent_key",
      selection: { kind: "existing", api_key_id: "key" },
    });
  });
  it("reviews a new key draft without minting before the final action", async () => {
    mintResponses();
    mount(<LoginAgentKeyPage mint />);
    await click("Restricted Agent Key");
    fireEvent.click(screen.getByRole("radio", { name: "Create a new key" }));
    await click("Review permissions");
    expect(
      screen.getByText("Confirm effective permissions"),
    ).toBeInTheDocument();
    expect(mocks.post).not.toHaveBeenCalledWith(
      "/auth/login-code",
      expect.anything(),
    );
    await click("Generate restricted login code");
    expect(mocks.post).toHaveBeenCalledWith(
      "/auth/login-code",
      expect.objectContaining({
        auth_kind: "agent_key",
        selection: expect.objectContaining({
          kind: "new",
          name: "CLI Agent",
          allow_all_services: false,
          allow_auto_connected_services: false,
        }),
      }),
    );
  });
  it("does not prefill the mint page", () => {
    mocks.search = { user_code: "garbage" };
    mocks.auth.signedIn = false;
    mount(<LoginAgentKeyPage mint />);
    expect(
      screen.getByRole("link", { name: "Approve on this computer" }),
    ).toHaveAttribute("href", "/login?return_to=%2Flogin%2Fcode");
    expect(
      screen.queryByRole("button", { name: "Enter another code" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("User code")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(mocks.preview).not.toHaveBeenCalled();
    expect(mocks.post).not.toHaveBeenCalled();
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
    mount(<LoginCredentialsSection keyId="key" canWrite />);
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
