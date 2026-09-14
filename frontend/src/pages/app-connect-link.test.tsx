import { StrictMode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "@/lib/api-client";
import type {
  AppConnectItem,
  AppConnectLink,
} from "@/schemas/app-connect-links";
import { AppConnectLinkPage } from "./app-connect-link";

const mocks = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  navigate: vi.fn(),
  auth: { user: { id: "person" }, isAuthenticated: true, isLoading: false },
}));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<object>()),
  api: { get: mocks.get, post: mocks.post },
}));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: () => mocks.auth }));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ linkId: "link" }),
  useNavigate: () => mocks.navigate,
}));

function item(overrides: Partial<AppConnectItem> = {}): AppConnectItem {
  return {
    requirement_id: "github",
    label: "GitHub",
    optional: false,
    state: "unknown",
    readiness: "unknown",
    user_service_id: "service",
    slug: "my-github",
    resource_uri: "https://nyxid.test/api/v1/proxy/s/my-github",
    owner_id: "person",
    connect_link_id: null,
    reason_code: null,
    claim: "The credential identifies a GitHub user.",
    validated_at: null,
    valid_until: null,
    granted_to_caller: false,
    catalog_slugs: ["api-github"],
    required_scopes: [],
    choices: [
      {
        user_service_id: "service",
        slug: "my-github",
        catalog_slug: "api-github",
        owner_id: "person",
      },
    ],
    ...overrides,
  };
}
function link(items = [item()]): AppConnectLink {
  return {
    id: "link",
    oauth_client_id: "app",
    client_name: "App A",
    handoff_blurb: "<b>Connect your accounts</b>",
    destination: "app.example",
    requirements_version: 1,
    status: "in_progress",
    expires_at: new Date(Date.now() + 1_800_000).toISOString(),
    items,
    callback_url: null,
    grant_update_required: false,
  };
}
function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const tree = () => (
    <StrictMode>
      <QueryClientProvider client={client}>
        <AppConnectLinkPage />
      </QueryClientProvider>
    </StrictMode>
  );
  const view = render(tree());
  return { ...view, rerenderSession: () => view.rerender(tree()) };
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.auth = {
    user: { id: "person" },
    isAuthenticated: true,
    isLoading: false,
  };
  window.history.replaceState(null, "", "/connect/app/link");
  mocks.get.mockResolvedValue(link());
  mocks.post.mockResolvedValue(link());
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("App Connect Link hosted page", () => {
  it("redeems once under StrictMode, scrubs the capability and renders fixed text branding without a probe", async () => {
    window.history.replaceState(
      null,
      "",
      "/connect/app/link#t=page-capability",
    );
    mount();
    await screen.findByText("App A");
    await waitFor(() => expect(window.location.hash).toBe(""));
    expect(mocks.post).toHaveBeenCalledExactlyOnceWith(
      "/app-connect-links/link/redeem",
      { capability: "page-capability" },
    );
    expect(mocks.get).not.toHaveBeenCalled();
    expect(
      screen.getByText("<b>Connect your accounts</b>").querySelector("b"),
    ).toBeNull();
    expect(
      screen.getByText("Secured by NyxID · app.example"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });
  it("resumes with one authenticated read and only checks on explicit click; clicks share the throttle", async () => {
    let now = Date.now();
    vi.spyOn(Date, "now").mockImplementation(() => now);
    mount();
    await screen.findByText("App A");
    expect(mocks.get).toHaveBeenCalledExactlyOnceWith(
      "/app-connect-links/link",
    );
    expect(mocks.post).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Re-check" }));
    await waitFor(() => expect(mocks.post).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Not now" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    expect(mocks.post).toHaveBeenCalledTimes(1);
    now += 751;
    fireEvent.click(screen.getByRole("button", { name: "Not now" }));
    await waitFor(() => expect(mocks.post).toHaveBeenCalledTimes(2));
    expect(mocks.post.mock.calls.map((c) => c[0])).toEqual([
      "/app-connect-links/link/items/github/validate",
      "/app-connect-links/link/cancel",
    ]);
  });
  it("unauthenticated visitors go to login with the capability in return_to and make no API calls", async () => {
    mocks.auth.isAuthenticated = false;
    window.history.replaceState(
      null,
      "",
      "/connect/app/link#t=page-capability",
    );
    mount();
    await waitFor(() => expect(mocks.navigate).toHaveBeenCalled());
    expect(mocks.navigate.mock.calls[0]?.[0]).toEqual({
      to: "/login",
      search: { return_to: window.location.href },
    });
    expect(mocks.get).not.toHaveBeenCalled();
    expect(mocks.post).not.toHaveBeenCalled();
  });
  it("already redeemed capability falls back to a subject-bound read", async () => {
    window.history.replaceState(null, "", "/connect/app/link#t=used");
    mocks.post.mockRejectedValue(
      new ApiError(404, {
        error: "not_found",
        error_code: 12001,
        message: "App connect link not found",
      }),
    );
    mount();
    await screen.findByText("App A");
    expect(mocks.get).toHaveBeenCalledExactlyOnceWith(
      "/app-connect-links/link",
    );
    expect(mocks.post).toHaveBeenCalledTimes(1);
  });
  it("never enables a disabled connection and includes no-credential services without requests", async () => {
    mocks.get.mockResolvedValue(
      link([
        item({ readiness: "disabled", state: "unmet" }),
        item({
          requirement_id: "included",
          label: "Included service",
          readiness: "included",
          state: "met",
          claim: null,
        }),
      ]),
    );
    mount();
    await screen.findByText("Disabled, enable to use.");
    expect(screen.getByText("Included", { exact: true })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Re-check" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Connect" })).toBeDisabled();
    expect(mocks.post).not.toHaveBeenCalled();
  });
  it("an elapsed check disables Continue and an abort keeps its reason visible", async () => {
    mocks.get.mockResolvedValue(
      link([
        item({
          state: "met",
          readiness: "met",
          validated_at: new Date(Date.now() - 301_000).toISOString(),
          valid_until: new Date(Date.now() - 1_000).toISOString(),
        }),
        item({
          requirement_id: "abort",
          label: "Other connection",
          reason_code: "attempt_superseded",
        }),
      ]),
    );
    mount();
    await screen.findByText("Check expired");
    expect(
      screen.getByText(
        "The connection changed while it was being checked. Check again.",
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });
  it("ready returns only to the server callback", async () => {
    const initial = link([item({ state: "met", readiness: "included" })]);
    mocks.get.mockResolvedValue(initial);
    const callback =
      "https://app.example/callback?status=completed&state=app-state&app_connect_link_id=link&grant_update_required=true";
    mocks.post.mockResolvedValue({
      ...initial,
      status: "completed",
      callback_url: callback,
      grant_update_required: true,
    });
    const assign = vi
      .spyOn(window.location, "assign")
      .mockImplementation(() => {});
    mount();
    await screen.findByText("App A");
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    await waitFor(() =>
      expect(assign).toHaveBeenCalledExactlyOnceWith(callback),
    );
    expect(mocks.post).toHaveBeenCalledExactlyOnceWith(
      "/app-connect-links/link/ready",
      {},
    );
  });
  it("uses the selected catalog slug for scope repair, including a custom service slug", async () => {
    mocks.get.mockResolvedValue(
      link([
        item({
          readiness: "needs_reauth",
          state: "failed",
          catalog_slugs: ["other", "api-github"],
          required_scopes: ["repo"],
        }),
      ]),
    );
    mocks.post.mockImplementation(async (path: string) => {
      if (path.endsWith("/reauthorize"))
        return {
          id: "child",
          token: "child-capability",
          service_name: "GitHub",
          service_slug: "api-github",
          connect_method: "oauth",
          auth_key_name: "Authorization",
          credential_mode: "admin",
          has_platform_oauth_credentials: true,
          requires_gateway_url: false,
          api_key_url: null,
          api_key_instructions: null,
        };
      return {
        id: "child",
        status: "oauth_required",
        authorization_url: "https://github.com/login/oauth/authorize",
        user_service_id: null,
        callback_url: null,
      };
    });
    vi.spyOn(window.location, "assign").mockImplementation(() => {});
    mount();
    await screen.findByText("Required permissions: repo");
    fireEvent.click(screen.getByRole("button", { name: "Reauthorize" }));
    await waitFor(() =>
      expect(mocks.post).toHaveBeenCalledWith("/connect-links/complete", {
        token: "child-capability",
      }),
    );
    expect(mocks.post).toHaveBeenCalledWith(
      "/app-connect-links/link/items/github/reauthorize",
      { service_slug: "api-github" },
    );
  });
  it("changes in authenticated subject never display the previous subject's cached session", async () => {
    const view = mount();
    await screen.findByText("App A");
    mocks.auth.user = { id: "other-person" };
    mocks.get.mockRejectedValue(new Error("Not found"));
    await act(async () => view.rerenderSession());
    expect(screen.queryByText("App A")).not.toBeInTheDocument();
  });
});
