import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
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
  query: "?user_code=ABCD-EFGH",
}));
vi.mock("@/lib/api-client", () => ({
  api: { post: mocks.post, get: mocks.get },
  apiClient: mocks.preview,
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: () => ({
    user: { id: "user" },
    isAuthenticated: mocks.auth,
    isLoading: false,
  }),
}));
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
  mocks.query = "?user_code=ABCD-EFGH";
  inventory = loginInventory();
  preview = makePreview();
  mocks.preview.mockImplementation(async () => preview);
  mocks.post.mockImplementation(async (path: string) =>
    path.endsWith("/options")
      ? structuredClone(inventory.options)
      : { ok: true },
  );
  mocks.get.mockImplementation(async (path: string) =>
    path === "/keys"
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
  await click("This is my request — continue");
  if (flow === "device")
    fireEvent.click(
      screen.getByRole("radio", { name: /Restricted Agent Key/ }),
    );
  await click("Continue to approval");
}
const approvals = () =>
  mocks.post.mock.calls.filter(([path]) => String(path).includes("/approve"));
describe("three-step device approval", () => {
  it("creates a NyxID account-permission key without any service selection", async () => {
    mocks.query +=
      "&login_type=agent&key_source=new&key_name=Account+reader&permissions=read";
    await scope();
    await click("Create new Agent Key");
    expect(
      screen.getByRole("region", { name: "Connections to grant" }),
    ).toHaveTextContent("0 selected");
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
  it("previews explicitly, shows actual requester details, and approves full account once", async () => {
    mount();
    expect(mocks.preview).not.toHaveBeenCalled();
    await click("Continue");
    expect(screen.getByText("203.0.113.5")).toBeVisible();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("img", { name: /QR/ })).not.toBeInTheDocument();
    await click("This is my request — continue");
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
    await click("This is my request — continue");
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
    const link = screen.getByRole("link", {
      name: "Verify identity to continue",
    });
    const returned = new URL(
      link.getAttribute("href")!,
      "https://nyx.test",
    ).searchParams.get("return_to")!;
    expect(returned).toContain("key_name=My+Agent");
    expect(returned).toContain("user_code=ABCDEFGH");
    expect(returned).toContain("service_permissions=github%3A%3Arepo%3Aread");
    expect(mocks.post).not.toHaveBeenCalled();
  });
  it.each(["device", "agent-key"] as const)(
    "approves an existing key via the %s protocol",
    async (flow) => {
      mocks.query +=
        "&login_type=agent&permissions=read,proxy&service_permissions=github::repo:read";
      await scope(flow);
      expect(screen.getByText("Exact match")).toBeVisible();
      fireEvent.click(screen.getByRole("radio", { name: "Reader" }));
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
    fireEvent.click(screen.getByText("Key settings and actual NyxID grant"));
    expect(
      screen.getByRole("button", { name: "January 31, 2026" }),
    ).toBeVisible();
    expect(
      screen.getByText(new Date("2026-01-31T23:59:59.000Z").toLocaleString()),
    ).toBeVisible();
    expect(
      screen.getByRole("button", {
        name: "Remove connection GitHub personal",
      }),
    ).toBeVisible();
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
    fireEvent.click(screen.getByRole("radio", { name: "Reader" }));
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
    fireEvent.click(screen.getByText("Key settings and actual NyxID grant"));
    fireEvent.click(
      screen.getByRole("checkbox", {
        name: "Allow all auto-connected platform services (includes ones added later)",
      }),
    );
    expect(
      screen.getByText(
        "All current and future auto-connected platform services",
      ),
    ).toBeVisible();
    expect(
      screen.getByRole("region", { name: "Connections to grant" }),
    ).toHaveTextContent("Platform user");
    expect(
      screen.getByRole("region", { name: "Connections to grant" }),
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
    expect(screen.getByRole("radio", { name: "Reader" })).toBeVisible();
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
    expect(screen.getByLabelText("User code")).toHaveValue("1234-5678");
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
  it("shows network failure and allows an explicit retry", async () => {
    mocks.preview.mockRejectedValueOnce(new Error("Unavailable"));
    mount();
    await click("Continue");
    expect(screen.getByText("Unavailable")).toBeVisible();
    await click("Continue");
    expect(screen.getByText("workstation")).toBeVisible();
  });
});
