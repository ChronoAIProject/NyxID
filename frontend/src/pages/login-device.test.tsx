import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LoginDevicePage } from "./login-device";
import { LoginAgentKeyPage } from "./login-agent-key";
import { loginInventory } from "@/lib/__fixtures__/login-inventory";
const mocks = vi.hoisted(() => ({
  post: vi.fn(),
  get: vi.fn(),
  preview: vi.fn(),
  auth: true,
  userId: "user",
  logout: vi.fn(),
  checkAuth: vi.fn(),
  query: "?user_code=ABCD-EFGH",
  config: { email_auth_enabled: true, social_providers: [] as string[] },
}));
vi.mock("@/lib/api-client", () => ({
  api: { post: mocks.post, get: mocks.get },
  apiClient: mocks.preview,
}));
vi.mock("@/stores/auth-store", () => {
  const state = () => ({
    user: { id: mocks.userId },
    isAuthenticated: mocks.auth,
    isLoading: false,
    logout: mocks.logout,
    checkAuth: mocks.checkAuth,
  });
  return { useAuthStore: Object.assign(state, { getState: state }) };
});
vi.mock("@tanstack/react-router", () => ({
  useLocation: ({
    select,
  }: {
    select: (v: { searchStr: string }) => unknown;
  }) => select({ searchStr: mocks.query }),
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
let inventory = loginInventory();
let preview = makePreview();
let client: QueryClient;
function makePreview() {
  return {
    supports_grant_choice: true,
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
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
  vi.clearAllMocks();
  mocks.auth = true;
  mocks.userId = "user";
  mocks.query = "?user_code=ABCD-EFGH";
  mocks.config = { email_auth_enabled: true, social_providers: [] };
  sessionStorage.clear();
  window.history.replaceState(null, "", "/");
  mocks.logout.mockResolvedValue(undefined);
  mocks.checkAuth.mockResolvedValue(undefined);
  inventory = loginInventory();
  preview = makePreview();
  mocks.preview.mockImplementation(async () => preview);
  mocks.post.mockImplementation(async (path: string) =>
    path.endsWith("/options")
      ? structuredClone(inventory.options)
      : { ok: true },
  );
  mocks.get.mockImplementation(async (path: string) =>
    path === "/public/config"
      ? mocks.config
      : path === "/keys"
        ? { keys: structuredClone(inventory.connections) }
        : { entries: inventory.catalog },
  );
  client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
});
afterEach(() => {
  cleanup();
  client.clear();
  vi.useRealTimers();
});
function mount(flow: "device" | "agent-key" = "device") {
  return render(
    <QueryClientProvider client={client}>
      {flow === "device" ? <LoginDevicePage /> : <LoginAgentKeyPage />}
    </QueryClientProvider>,
  );
}
async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
  });
}
async function scope(flow: "device" | "agent-key" = "device") {
  mount(flow);
  await click("Continue");
  await click("Continue");
  if (flow === "device")
    fireEvent.click(
      screen.getByRole("radio", { name: /Restricted Agent Key/ }),
    );
  await click("Continue to approval");
}
const approvals = () =>
  mocks.post.mock.calls.filter(([path]) => String(path).includes("/approve"));
describe("three-step device approval", () => {
  it("preserves collapsed details through step navigation and expands them on Review request", async () => {
    mocks.query += "&show_details=true";
    mount();
    await act(async () => {});
    const details = screen
      .getByText("Request details", { selector: "summary" })
      .closest("details")!;
    expect(details).toHaveAttribute("open");
    await act(async () => {
      fireEvent.click(details.querySelector("summary")!);
    });
    expect(details).not.toHaveAttribute("open");
    await click("Continue");
    expect(
      screen.getByRole("button", { name: "Step 1: Verify request Completed" }),
    ).toBeEnabled();
    await click("Step 1: Verify request Completed");
    expect(details).not.toHaveAttribute("open");
    expect(
      screen.getByRole("button", { name: "Step 1: Verify request Reviewing" }),
    ).toHaveAttribute("aria-current", "step");
    await click("Continue");
    await click("Review request");
    expect(details).toHaveAttribute("open");
    expect(details).toBeVisible();
    await act(async () => {
      fireEvent.click(details.querySelector("summary")!);
    });
    expect(details).not.toHaveAttribute("open");
    await click("Continue");
    await click("Continue to approval");
    await click("Review request");
    expect(details).toHaveAttribute("open");
    expect(details).toBeVisible();
    expect(approvals()).toEqual([]);
  });
  it("puts Continue before Deny on step one and Cancel after the primary action on later steps", async () => {
    mount();
    await act(async () => {});
    const deny = screen.getByRole("button", { name: "Deny request" });
    expect(
      within(deny.parentElement!)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Continue", "Deny request"]);
    await click("Continue");
    const stepTwoDeny = screen.getByRole("button", { name: "Cancel request" });
    expect(
      within(stepTwoDeny.parentElement!)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Continue to approval", "Cancel request"]);
    await click("Continue to approval");
    const stepThreeCancel = screen.getByRole("button", {
      name: "Cancel request",
    });
    expect(
      within(stepThreeCancel.parentElement!)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Approve full account access", "Cancel request"]);
  });
  it("retains the latest step when reviewing completed steps", async () => {
    await scope();
    await click("Step 1: Verify request Completed");
    expect(
      screen.getByRole("button", { name: "Step 1: Verify request Reviewing" }),
    ).toHaveAttribute("aria-current", "step");
    expect(
      screen.getByRole("button", { name: "Step 2: Choose access Completed" }),
    ).toBeEnabled();
    const latest = screen.getByRole("button", {
      name: "Step 3: Scope & approval In progress",
    });
    expect(latest).toBeEnabled();
    expect(latest).not.toHaveAttribute("aria-current");
    await click("Step 2: Choose access Completed");
    expect(
      screen.getByRole("button", { name: "Step 2: Choose access Reviewing" }),
    ).toHaveAttribute("aria-current", "step");
    await click("Step 3: Scope & approval In progress");
    expect(latest).toHaveAttribute("aria-current", "step");
    expect(
      screen.getByRole("textbox", { name: "Search permissions" }),
    ).toBeVisible();
    expect(approvals()).toEqual([]);
  });
  it("cancels a new-key draft without submitting or creating a key", async () => {
    await scope();
    await click("Create new Agent Key");
    const cancel = screen.getByRole("button", { name: "Cancel request" });
    expect(
      within(cancel.parentElement!)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Create & continue", "Cancel request"]);
    await click("Cancel request");
    expect(screen.getByText("Request denied")).toBeVisible();
    expect(approvals()).toEqual([]);
    expect(mocks.post).toHaveBeenCalledWith("/auth/device/deny", {
      user_code: "ABCDEFGH",
    });
  });
  it.each([false, true])(
    "shows link filter reset only when permissions were supplied: %s",
    async (hinted) => {
      if (hinted) mocks.query += "&permissions=read";
      await scope();
      const reset = screen.queryByRole("button", {
        name: "Reset to link filters",
      });
      if (hinted) {
        expect(reset).toBeVisible();
        await click("Clear filters");
        expect(screen.getByRole("status")).toHaveTextContent("0 selected");
        await click("Reset to link filters");
        expect(screen.getByRole("status")).toHaveTextContent("1 selected");
      } else expect(reset).not.toBeInTheDocument();
    },
  );
  it("offers only enabled identity methods and reveals email fields after selection", async () => {
    mocks.auth = false;
    mocks.config = { email_auth_enabled: true, social_providers: ["google"] };
    mount();
    await act(async () => {});
    expect(
      screen.getByRole("button", { name: "Continue with Google" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Continue with GitHub" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
    await click("Continue with email");
    expect(screen.getByLabelText("Password")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Continue with Google" }),
    ).not.toBeInTheDocument();
    await click("Other sign-in methods");
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Continue with Google" }),
    ).toBeVisible();
    expect(mocks.post).not.toHaveBeenCalled();
  });
  it("never offers email when it is disabled", async () => {
    mocks.auth = false;
    mocks.config = { email_auth_enabled: false, social_providers: ["github"] };
    mount();
    await act(async () => {});
    expect(
      screen.getByRole("button", { name: "Continue with GitHub" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Continue with email" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
  });
  it.each([false, true])(
    "verifies inline with keep_signed_in=%s, then requires explicit approval",
    async (keep) => {
      mocks.auth = false;
      window.history.replaceState(
        null,
        "",
        "/login/device?user_code=ABCD-EFGH",
      );
      mocks.query = "";
      const identity = {
        id: "11111111-1111-4111-8111-111111111111",
        flow: "device",
        user_code: "ABCDEFGH",
        keep_signed_in: keep,
        verified: false,
        mfa_required: false,
        user: null,
        expires_at: "2026-01-01T00:10:00Z",
      };
      mocks.preview.mockImplementation(async (path: string) => {
        if (path === "/auth/approval") return identity;
        if (path.endsWith("/password"))
          return {
            ...identity,
            verified: true,
            user: {
              id: "verified-human",
              email: "human@example.com",
              display_name: null,
            },
          };
        if (path.endsWith("/approve")) return { ok: true };
        return preview;
      });
      mount();
      await act(async () => {});
      expect(screen.getByText("203.0.113.5")).toBeVisible();
      if (keep)
        await click("Keep me signed in Also sign in to NyxID in this browser.");
      fireEvent.change(screen.getByLabelText("Email"), {
        target: { value: "human@example.com" },
      });
      fireEvent.change(screen.getByLabelText("Password"), {
        target: { value: "Local-test-password1!" },
      });
      await click(keep ? "Sign in & continue" : "Verify & continue");
      expect(mocks.preview).toHaveBeenCalledWith(
        "/auth/approval",
        expect.objectContaining({
          body: { flow: "device", user_code: "ABCDEFGH", keep_signed_in: keep },
        }),
      );
      expect(window.location.pathname).toBe("/login/device");
      expect(window.location.search).toBe("?user_code=ABCD-EFGH");
      expect(mocks.auth).toBe(false);
      expect(mocks.checkAuth).toHaveBeenCalledTimes(keep ? 1 : 0);
      expect(
        mocks.post.mock.calls.some(([path]) => path === "/auth/login"),
      ).toBe(false);
      expect(
        mocks.preview.mock.calls.some(([path]) => path.endsWith("/approve")),
      ).toBe(false);
      await click("Continue to approval");
      await click("Approve full account access");
      expect(mocks.preview).toHaveBeenCalledWith(
        `/auth/approval/${identity.id}/approve`,
        expect.objectContaining({ body: {}, method: "POST" }),
      );
      expect(
        screen.getByText("Approved — return to the requesting device"),
      ).toBeVisible();
      expect(
        sessionStorage.getItem("nyxid-approval:device:ABCDEFGH"),
      ).toBeNull();
      expect(screen.queryByText("Change account")).not.toBeInTheDocument();
    },
  );
  it("requires the second factor inside the same request before showing access choices", async () => {
    mocks.auth = false;
    const identity = {
      id: "22222222-2222-4222-8222-222222222222",
      flow: "device",
      user_code: "ABCDEFGH",
      keep_signed_in: false,
      verified: false,
      mfa_required: false,
      user: null,
      expires_at: "2026-01-01T00:10:00Z",
    };
    mocks.preview.mockImplementation(async (path: string) => {
      if (path === "/auth/approval") return identity;
      if (path.endsWith("/password"))
        return { ...identity, mfa_required: true };
      if (path.endsWith("/mfa"))
        return {
          ...identity,
          verified: true,
          user: {
            id: "mfa-human",
            email: "human@example.com",
            display_name: null,
          },
        };
      return preview;
    });
    mount();
    await act(async () => {});
    fireEvent.change(screen.getByLabelText("Email"), {
      target: { value: "human@example.com" },
    });
    fireEvent.change(screen.getByLabelText("Password"), {
      target: { value: "Local-test-password1!" },
    });
    await click("Verify & continue");
    expect(
      screen.queryByRole("button", { name: "Continue to approval" }),
    ).not.toBeInTheDocument();
    expect(screen.getByLabelText("Authenticator code")).toBeVisible();
    fireEvent.change(screen.getByLabelText("Authenticator code"), {
      target: { value: "123456" },
    });
    await click("Verify & continue");
    expect(
      screen.getByRole("button", { name: "Continue to approval" }),
    ).toBeVisible();
    expect(mocks.preview).toHaveBeenCalledWith(
      `/auth/approval/${identity.id}/mfa`,
      expect.objectContaining({ body: { code: "123456" } }),
    );
    expect(approvals()).toEqual([]);
  });
  it("clears both the local proof and browser session when changing a persistent identity", async () => {
    mocks.auth = false;
    const identity = {
      id: "33333333-3333-4333-8333-333333333333",
      flow: "device",
      user_code: "ABCDEFGH",
      keep_signed_in: true,
      verified: true,
      mfa_required: false,
      user: { id: "human", email: "human@example.com", display_name: null },
      expires_at: "2026-01-01T00:10:00Z",
    };
    sessionStorage.setItem(
      "nyxid-approval:device:ABCDEFGH",
      JSON.stringify(identity),
    );
    mocks.preview.mockImplementation(async (path: string) =>
      path.startsWith("/auth/approval/") ? identity : preview,
    );
    mount();
    await act(async () => {});
    await click("Sign out of this browser");
    expect(mocks.preview).toHaveBeenCalledWith(
      `/auth/approval/${identity.id}`,
      expect.objectContaining({ method: "DELETE" }),
    );
    expect(mocks.logout).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Verify your identity")).toBeVisible();
    expect(sessionStorage.getItem("nyxid-approval:device:ABCDEFGH")).toBeNull();
  });
  it("returns to identity verification when the browser session expires during review", async () => {
    const view = mount();
    await click("Continue");
    await click("Continue");
    await click("Continue to approval");
    expect(
      screen.getByRole("button", { name: "Approve full account access" }),
    ).toBeEnabled();
    mocks.auth = false;
    view.rerender(
      <QueryClientProvider client={client}>
        <LoginDevicePage />
      </QueryClientProvider>,
    );
    expect(
      screen.getByRole("button", { name: /Only for this request/ }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Approve full account access" }),
    ).not.toBeInTheDocument();
    expect(approvals()).toEqual([]);
    mocks.auth = true;
    view.rerender(
      <QueryClientProvider client={client}>
        <LoginDevicePage />
      </QueryClientProvider>,
    );
    expect(screen.getByRole("button", { name: "Continue" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Approve full account access" }),
    ).not.toBeInTheDocument();
    expect(approvals()).toEqual([]);
  });
  it("discards pending connection discovery when the approving identity changes", async () => {
    mocks.query += "&login_type=agent";
    let resolveOptions!: (value: unknown) => void;
    mocks.post.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveOptions = resolve;
        }),
    );
    const view = mount();
    await click("Continue");
    await click("Continue");
    await act(async () => {
      fireEvent.click(
        screen.getByRole("button", { name: "Continue to approval" }),
      );
    });
    mocks.userId = "different-human";
    view.rerender(
      <QueryClientProvider client={client}>
        <LoginDevicePage />
      </QueryClientProvider>,
    );
    await act(async () => {
      resolveOptions(structuredClone(inventory.options));
    });
    expect(screen.getByRole("button", { name: "Continue" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Create new Agent Key" }),
    ).not.toBeInTheDocument();
    expect(approvals()).toEqual([]);
  });
  it("creates a NyxID account-permission key without any service selection", async () => {
    mocks.query +=
      "&login_type=agent&key_source=new&key_name=Account+reader&permissions=read";
    await scope();
    await click("Create new Agent Key");
    expect(
      screen.getByRole("region", { name: "Authorize access" }),
    ).toHaveTextContent("Account reader");
    expect(screen.queryByLabelText("GitHub access")).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Create & continue" }),
    ).toBeEnabled();
    expect(approvals()).toEqual([]);
    await click("Create & continue");
    expect(approvals()).toEqual([
      [
        "/auth/device/approve-agent-key",
        expect.objectContaining({
          selection: expect.objectContaining({
            name: "Account reader",
            scopes: "read",
            allowed_service_ids: [],
            connection_snapshots: [],
            allow_all_services: false,
          }),
        }),
      ],
    ]);
  });
  it("previews linked requests, shows actual requester details, and approves full account once", async () => {
    mount();
    expect(mocks.preview).toHaveBeenCalledTimes(1);
    await click("Continue");
    expect(screen.getByText("203.0.113.5")).toBeVisible();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("img", { name: /QR/ })).not.toBeInTheDocument();
    await click("Continue");
    await click("Continue to approval");
    expect(approvals()).toEqual([]);
    await click("Approve full account access");
    expect(approvals()).toEqual([
      ["/auth/device/approve", { user_code: "ABCDEFGH" }],
    ]);
    expect(
      screen.getByText("Approved — return to the requesting device"),
    ).toBeVisible();
  });
  it("keeps a legacy device request account-only and an agent request restricted", async () => {
    preview.supports_grant_choice = false;
    mount();
    await click("Continue");
    await click("Continue");
    expect(
      screen.queryByRole("radio", { name: /Restricted/ }),
    ).not.toBeInTheDocument();
    cleanup();
    await scope("agent-key");
    expect(
      screen.queryByRole("radio", { name: /Full account/ }),
    ).not.toBeInTheDocument();
  });
  it("preserves request hints through identity verification without approving", async () => {
    mocks.auth = false;
    mocks.query +=
      "&login_type=agent&key_source=new&key_name=My+Agent&permissions=read,proxy&service_permissions=github::repo:read";
    mount();
    await click("Continue");
    expect(
      screen.getByRole("button", { name: /Only for this request/ }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /Keep me signed in/ }),
    ).toHaveAttribute("aria-pressed", "false");
    expect(
      screen.queryByRole("link", { name: /Verify identity/ }),
    ).not.toBeInTheDocument();
    expect(mocks.post).not.toHaveBeenCalled();
  });
  it.each(["device", "agent-key"] as const)(
    "approves an existing key via the %s protocol",
    async (flow) => {
      mocks.query +=
        "&login_type=agent&permissions=read,proxy&service_permissions=github::repo:read";
      await scope(flow);
      expect(screen.getByText("Exact match")).toBeVisible();
      fireEvent.click(screen.getByRole("button", { name: "Reader" }));
      await click("Approve access with this key");
      expect(approvals()).toEqual([
        [
          `/auth/${flow}/${flow === "device" ? "approve-agent-key" : "approve"}`,
          {
            user_code: "ABCDEFGH",
            selection: {
              kind: "existing",
              api_key_id: "key",
              permission_snapshot: "k".repeat(64),
            },
          },
        ],
      ]);
    },
  );
  it("creates a hinted key with the exact connection only on final approval", async () => {
    mocks.query +=
      "&login_type=agent&key_source=new&key_name=Build+agent&platform=codex&expiry_days=30&permissions=read,proxy&service_permissions=github::repo:read";
    await scope();
    await click("Create new Agent Key");
    expect(screen.getByLabelText("Name")).toHaveValue("Build agent");
    fireEvent.click(screen.getByText("Customize"));
    expect(
      screen.getByRole("button", { name: "January 31, 2026" }),
    ).toBeVisible();
    expect(
      screen.getByText(new Date("2026-01-31T23:59:59.000Z").toLocaleString()),
    ).toBeVisible();
    fireEvent.click(
      screen.getByLabelText("GitHub access").querySelector("summary")!,
    );
    expect(
      screen.getByRole("checkbox", {
        name: "Grant connection GitHub personal",
      }),
    ).toBeChecked();
    expect(approvals()).toEqual([]);
    await click("Create & continue");
    expect(approvals()[0]).toEqual([
      "/auth/device/approve-agent-key",
      expect.objectContaining({
        selection: expect.objectContaining({
          kind: "new",
          name: "Build agent",
          scopes: "read proxy",
          allowed_service_ids: ["svc"],
          platform: "codex",
          expires_at: "2026-01-31T23:59:59.000Z",
        }),
      }),
    ]);
  });
  it("requires new review if connection scopes change before approval", async () => {
    await scope();
    fireEvent.click(screen.getByRole("button", { name: "Reader" }));
    inventory.connections[0]!.granted_scopes!.push("repo:write");
    await click("Approve access with this key");
    expect(
      screen.getByText(/The key or connection access changed/),
    ).toBeVisible();
    expect(approvals()).toEqual([]);
  });
  it("discloses durable platform access and sends snapshots for current same-owner implied rows", async () => {
    inventory.options.personal_owner_id = "user";
    for (const owner of ["user", "org"]) {
      inventory.options.services.push({
        id: `platform-${owner}`,
        name: `Platform ${owner}`,
        owner_id: owner,
        auto_connected: true,
      });
      inventory.connections.push({
        ...inventory.connections[0]!,
        id: `platform-${owner}`,
        label: `Platform ${owner}`,
        catalog_service_slug: "platform",
        credential_binding: "platform",
        granted_scopes: null,
        permission_snapshot: owner.repeat(64).slice(0, 64),
      });
    }
    inventory.options.connections = inventory.connections;
    inventory.catalog.push({ slug: "platform", name: "Platform" });
    mocks.query +=
      "&login_type=agent&key_source=new&key_name=Platform+agent&permissions=read,proxy&service_permissions=github::repo:read";
    await scope();
    await click("Create new Agent Key");
    fireEvent.click(screen.getByText("Customize"));
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "Allow all auto-connected platform services (includes ones added later)",
      }),
    );
    expect(
      screen.getByText("Includes future platform services."),
    ).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Authorize access" }),
    ).toHaveTextContent("Platform user");
    expect(
      screen
        .getByRole("region", { name: "Authorize access" })
        .querySelector('details[aria-label="Platform access"] summary'),
    ).not.toHaveTextContent("Platform org");
    expect(approvals()).toEqual([]);
    await click("Create & continue");
    expect(approvals()[0]).toEqual([
      "/auth/device/approve-agent-key",
      expect.objectContaining({
        selection: expect.objectContaining({
          kind: "new",
          allow_auto_connected_services: true,
          allowed_service_ids: ["svc"],
          connection_snapshots: [
            { service_id: "svc", permission_snapshot: "c".repeat(64) },
            {
              service_id: "platform-user",
              permission_snapshot: "user".repeat(64).slice(0, 64),
            },
          ],
        }),
      }),
    ]);
  });
  it("blocks unknown permissions and supports explicit filter removal", async () => {
    mocks.query += "&login_type=agent&service_permissions=github::missing";
    await scope();
    expect(screen.getByText(/Unknown permissions or services:/)).toBeVisible();
    expect(
      screen.queryByRole("radio", { name: "Reader" }),
    ).not.toBeInTheDocument();
    await click("Clear filters");
    expect(screen.getByRole("button", { name: "Reader" })).toBeVisible();
  });
  it("denial is terminal even if an older poll returns pending", async () => {
    mount();
    await click("Continue");
    let resolve!: (value: typeof preview) => void;
    mocks.preview.mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    await click("Deny request");
    await act(async () => {
      resolve(preview);
      await vi.advanceTimersByTimeAsync(10000);
    });
    expect(screen.getByText("Request denied")).toBeVisible();
    expect(approvals()).toEqual([]);
    expect(mocks.preview).toHaveBeenCalledTimes(2);
  });
  it("ignores a stale request response after navigating to another link", async () => {
    let resolve!: (value: typeof preview) => void;
    mocks.preview.mockImplementationOnce(
      () =>
        new Promise((r) => {
          resolve = r;
        }),
    );
    mocks.preview.mockResolvedValue({
      ...preview,
      client_label: "new requester",
    });
    const view = mount();
    await click("Continue");
    mocks.query = "?user_code=12345678";
    view.rerender(
      <QueryClientProvider client={client}>
        <LoginDevicePage />
      </QueryClientProvider>,
    );
    await act(async () => {
      resolve(preview);
    });
    expect(screen.getByText("new requester")).toBeVisible();
    expect(screen.getByText("1234-5678")).toBeVisible();
    expect(screen.queryByText("workstation")).not.toBeInTheDocument();
  });
  it.each(["denied", "expired", "approved", "delivered"])(
    "stops at terminal request status %s",
    async (status) => {
      preview.status = status;
      mount();
      await click("Continue");
      expect(
        screen.queryByRole("button", { name: "Deny request" }),
      ).not.toBeInTheDocument();
      expect(mocks.post).not.toHaveBeenCalled();
    },
  );
  it.each(["device", "agent-key"] as const)(
    "offers a fresh %s code entry after expiry without carrying the old request hints",
    async (flow) => {
      mocks.query += "&login_type=agent&permissions=read&show_details=true";
      preview.status = "expired";
      mount(flow);
      await act(async () => {});
      expect(
        screen.getByRole("heading", { level: 1, name: "Request expired" }),
      ).toBeVisible();
      expect(
        screen.getByRole("link", { name: "Enter a new code" }),
      ).toHaveAttribute("href", `/login/${flow}`);
      expect(
        screen.queryByRole("navigation", { name: "Approval steps" }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("heading", { name: /Approve .*login/ }),
      ).not.toBeInTheDocument();
      expect(approvals()).toEqual([]);
    },
  );
  it("shows network failure and allows an explicit retry", async () => {
    mocks.preview.mockRejectedValueOnce(new Error("Unavailable"));
    mount();
    await click("Continue");
    expect(screen.getByText("Unavailable")).toBeVisible();
    await click("Continue");
    expect(screen.getByText("workstation")).toBeVisible();
  });
});
