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

const mocks = vi.hoisted(() => ({
  post: vi.fn(),
  get: vi.fn(),
  remove: vi.fn(),
  preview: vi.fn(),
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
    search: { return_to: string };
  }) => (
    <a href={`${to}?return_to=${encodeURIComponent(search.return_to)}`}>
      {children}
    </a>
  ),
}));

const key = {
  id: "key",
  name: "Shared key",
  key_prefix: "nyxid_ag_12345678",
  owner_type: "personal",
  owner_id: "user",
  owner_name: "Human",
  scopes: "read proxy",
  allow_all_services: false,
  allow_all_nodes: false,
  allowed_service_ids: [],
  allowed_node_ids: [],
  allowed_services: [],
  allowed_nodes: [],
  expires_at: null,
  rate_limit_per_second: null,
  rate_limit_burst: null,
  platform: "generic",
  created_now: false,
};
const issued = {
  request_id: "request-fixture",
  code: "JKLM-NPQR",
  expires_at: "2026-01-01T00:05:00Z",
};
let client: QueryClient;
let closed: boolean;
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  vi.clearAllMocks();
  mocks.auth.signedIn = true;
  closed = false;
  mocks.post.mockImplementation(async (path: string) =>
    path.endsWith("options")
      ? { keys: [key], services: [], nodes: [], orgs: [] }
      : issued,
  );
  mocks.remove.mockImplementation(async () => {
    closed = true;
    return { ok: true };
  });
  mocks.get.mockImplementation(async () => ({
    request_id: issued.request_id,
    status: closed ? "cancelled" : "pending",
    auth_kind: "agent_key",
    expires_at: issued.expires_at,
    redeemed_at: null,
    client_label: null,
    client_user_agent: null,
    client_ip: null,
    client_ip_attribution: "unavailable",
    can_revoke: false,
  }));
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});
function mount() {
  render(
    <QueryClientProvider client={client}>
      <LoginAgentKeyPage mint />
    </QueryClientProvider>,
  );
}
async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
    await vi.advanceTimersByTimeAsync(800);
  });
}
async function selectExisting() {
  await click("Restricted Agent Key");
  fireEvent.click(screen.getByRole("radio", { name: "Shared key" }));
  await click("Review permissions");
}

describe("One-time login code page", () => {
  it("starts with a grant choice, makes no request on mount, and links anonymous users back", () => {
    mocks.auth.signedIn = false;
    mount();
    expect(
      screen.getByRole("heading", { name: "One-time login code" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("User code")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Approve from your phone" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: "Approve on this computer" }),
    ).toHaveAttribute("href", "/login?return_to=%2Flogin%2Fcode");
    expect(mocks.post).not.toHaveBeenCalled();
    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.preview).not.toHaveBeenCalled();
  });

  it("mints account access only after confirmation and clears the mutation's code", async () => {
    mount();
    expect(mocks.post).not.toHaveBeenCalled();
    await click("Full account session");
    expect(screen.getByText("Confirm full account access")).toBeInTheDocument();
    expect(mocks.post).not.toHaveBeenCalled();
    await click("Generate account login code");
    expect(mocks.post.mock.calls).toEqual([
      ["/auth/login-code", { auth_kind: "account_session" }],
    ]);
    expect(screen.getByText(issued.code)).toBeInTheDocument();
    expect(
      JSON.stringify(
        client
          .getMutationCache()
          .getAll()
          .map((mutation) => mutation.state),
      ),
    ).not.toContain(issued.code);
  });

  it("loads code options with an empty body and confirms the restricted grant with its expiry", async () => {
    mount();
    await selectExisting();
    expect(mocks.post.mock.calls).toEqual([["/auth/login-code/options", {}]]);
    fireEvent.change(
      screen.getByLabelText("Login credential expiry (optional)"),
      {
        target: { value: "2026-02-01T12:30" },
      },
    );
    await click("Generate restricted login code");
    expect(mocks.post.mock.calls).toEqual([
      ["/auth/login-code/options", {}],
      [
        "/auth/login-code",
        {
          auth_kind: "agent_key",
          selection: { kind: "existing", api_key_id: "key" },
          credential_expires_at: new Date("2026-02-01T12:30").toISOString(),
        },
      ],
    ]);
    expect(screen.getByText(issued.code)).toBeInTheDocument();
    expect(mocks.preview).not.toHaveBeenCalled();
  });

  it("keeps account Reject and restricted Cancel local and retains the existing choice and expiry", async () => {
    mount();
    await click("Full account session");
    await click("Reject");
    expect(mocks.post).not.toHaveBeenCalled();
    await selectExisting();
    fireEvent.change(
      screen.getByLabelText("Login credential expiry (optional)"),
      {
        target: { value: "2026-02-01T12:30" },
      },
    );
    await click("Back");
    expect(screen.getByRole("radio", { name: "Shared key" })).toBeChecked();
    await click("Review permissions");
    await click("Cancel");
    await click("Restricted Agent Key");
    expect(screen.getByRole("radio", { name: "Shared key" })).toBeChecked();
    await click("Review permissions");
    expect(
      screen.getByLabelText("Login credential expiry (optional)"),
    ).toHaveValue("2026-02-01T12:30");
    expect(mocks.post.mock.calls).toEqual([
      ["/auth/login-code/options", {}],
      ["/auth/login-code/options", {}],
    ]);
  });

  it("shares the 750 ms throttle between opening account confirmation and minting", async () => {
    mount();
    fireEvent.click(
      screen.getByRole("button", { name: "Full account session" }),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(749);
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Generate account login code" }),
    );
    expect(mocks.post).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
      fireEvent.click(
        screen.getByRole("button", { name: "Generate account login code" }),
      );
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mocks.post).toHaveBeenCalledOnce();
  });

  it("retains a new-key draft through Back and generating another code", async () => {
    mount();
    await click("Restricted Agent Key");
    fireEvent.click(screen.getByRole("radio", { name: "Create a new key" }));
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Saved draft" },
    });
    await click("Review permissions");
    await click("Back");
    expect(screen.getByLabelText("Name")).toHaveValue("Saved draft");
    await click("Review permissions");
    await click("Generate restricted login code");
    expect(mocks.post).toHaveBeenCalledWith("/auth/login-code", {
      auth_kind: "agent_key",
      selection: expect.objectContaining({
        kind: "new",
        name: "Saved draft",
        allow_all_services: false,
        allow_all_nodes: false,
      }),
    });
    await click("Cancel code");
    await click("Generate another code");
    await click("Restricted Agent Key");
    expect(
      screen.getByRole("radio", { name: "Create a new key" }),
    ).not.toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: "Create a new key" }));
    expect(screen.getByLabelText("Name")).toHaveValue("Saved draft");
  });
});
