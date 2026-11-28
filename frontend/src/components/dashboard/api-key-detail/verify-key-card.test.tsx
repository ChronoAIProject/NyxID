import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { useAuthStore } from "@/stores/auth-store";
import type { ApiKey, User } from "@/types/api";
import {
  FIRST_PROXY_CALL_SUCCEEDED_KEY,
  useProxyOnboarding,
} from "@/hooks/use-proxy-onboarding";
import { VerifyKeyCard } from "./verify-key-card";

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: [{ id: "svc-1", slug: "api-github-2", is_active: true }],
  }),
}));

const key: ApiKey = {
  id: "ak-1",
  name: "GitHub agent",
  description: null,
  key_prefix: "nyxid_ag_test",
  scopes: "proxy",
  created_at: "2026-01-01T00:00:00Z",
  last_used_at: null,
  expires_at: null,
  is_active: true,
  allowed_service_ids: ["svc-1"],
  allowed_node_ids: [],
  allow_all_services: false,
  allow_all_nodes: false,
  allowed_services: [
    {
      id: "svc-1",
      slug: "api-github-2",
      label: "GitHub",
      catalog_service_name: "GitHub",
    },
  ],
  allowed_nodes: [],
  platform: null,
  callback_url: null,
  rate_limit_per_second: null,
  rate_limit_burst: null,
  bindings_count: 0,
  credential_source: { type: "personal" },
};
const marker = `${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-a`;

function proxied(status = 200) {
  return new Response(null, {
    status,
    headers: { "x-nyxid-agent-id": "ak-1" },
  });
}

function gatewayDenial(status = 404) {
  return Response.json(
    {
      error: status === 404 ? "not_found" : "forbidden",
      error_code: status === 404 ? 1003 : 1002,
    },
    { status },
  );
}

async function testKey() {
  const user = userEvent.setup();
  await user.type(screen.getByLabelText("Agent key"), "nyxid_ag_test-secret");
  await user.click(screen.getByRole("button", { name: "Test key" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Test key" })).toBeEnabled(),
  );
}

beforeEach(async () => {
  localStorage.clear();
  useAuthStore.setState({
    user: { id: "user-a" } as User,
    isAuthenticated: true,
  });
  await appI18n.changeLanguage("en");
});

it.each([403, 404])(
  "persists a real successful call without Dashboard mounted (denied %s)",
  async (status) => {
    const fetch = vi
      .spyOn(window, "fetch")
      .mockImplementation(async (url) =>
        String(url).includes("__nyxid_test_denied__")
          ? gatewayDenial(status)
          : proxied(),
      );
    const view = render(<VerifyKeyCard apiKey={key} />);
    await testKey();
    expect(fetch).toHaveBeenCalledWith(
      "/api/v1/proxy/s/api-github-2/user",
      expect.objectContaining({ method: "GET", redirect: "manual" }),
    );
    expect(screen.getByText("Gateway checks passed")).toBeInTheDocument();
    expect(screen.getAllByText(/End-to-end verified/)).toHaveLength(1);
    expect(screen.queryByText(/above/)).not.toBeInTheDocument();
    expect(localStorage.getItem(marker)).toBe("1");
    view.unmount();
    expect(
      renderHook(useProxyOnboarding).result.current.firstProxyCallSucceeded,
    ).toBe(true);
  },
);

it.each([301, 400, 401, 403, 404, 429, 500, 503])(
  "keeps scope evidence and provider HTTP %s diagnostic without activation",
  async (status) => {
    vi.spyOn(window, "fetch").mockImplementation(async (url) =>
      String(url).includes("__nyxid_test_denied__")
        ? gatewayDenial()
        : proxied(status),
    );
    render(<VerifyKeyCard apiKey={key} />);
    await testKey();
    expect(screen.getByText("Gateway checks passed")).toBeInTheDocument();
    expect(screen.getAllByText(/Your Agent Key works/)).toHaveLength(1);
    expect(localStorage.getItem(marker)).toBeNull();
  },
);

it("recognizes a gateway api_key_scope_forbidden/9000 denial", async () => {
  vi.spyOn(window, "fetch").mockImplementation(async (url) =>
    String(url).includes("__nyxid_test_denied__")
      ? Response.json(
          { error: "api_key_scope_forbidden", error_code: 9000 },
          { status: 403 },
        )
      : proxied(),
  );
  render(<VerifyKeyCard apiKey={key} />);
  await testKey();
  expect(screen.getByText("denied (expected)")).toBeInTheDocument();
  expect(screen.getByText("Gateway checks passed")).toBeInTheDocument();
});

it.each([
  "network",
  "401",
  "429",
  "500",
  "503",
  "headerless404",
  "provider403",
])(
  "does not report %s as an expected denial or accuse the gateway of leaking",
  async (failure) => {
    vi.spyOn(window, "fetch").mockImplementation(async (url) => {
      if (!String(url).includes("__nyxid_test_denied__")) return proxied();
      if (failure === "network") throw new TypeError("network");
      if (failure === "headerless404")
        return Response.json({ message: "upstream missing" }, { status: 404 });
      if (failure === "provider403") return proxied(403);
      return new Response(null, { status: Number(failure) });
    });
    render(<VerifyKeyCard apiKey={key} />);
    await testKey();
    expect(screen.getByText("Denial unverified")).toBeInTheDocument();
    expect(screen.queryByText("denied (expected)")).not.toBeInTheDocument();
    expect(screen.queryByText(/leaking access/)).not.toBeInTheDocument();
    // A real allowed success still completes activation independently.
    expect(localStorage.getItem(marker)).toBe("1");
  },
);

it("does not activate on a headerless allowed 200", async () => {
  vi.spyOn(window, "fetch").mockImplementation(async (url) =>
    String(url).includes("__nyxid_test_denied__")
      ? gatewayDenial()
      : new Response(null, { status: 200 }),
  );
  render(<VerifyKeyCard apiKey={key} />);
  await testKey();
  expect(screen.getByText("Access unverified")).toBeInTheDocument();
  expect(localStorage.getItem(marker)).toBeNull();
});

it("rejects another key's successful response on the selected key's detail", async () => {
  vi.spyOn(window, "fetch").mockImplementation(async (url) =>
    String(url).includes("__nyxid_test_denied__")
      ? gatewayDenial()
      : new Response(null, {
          status: 200,
          headers: { "x-nyxid-agent-id": "synthetic-key-b" },
        }),
  );
  render(<VerifyKeyCard apiKey={key} />);
  await testKey();
  expect(screen.getByText("Access unverified")).toBeInTheDocument();
  expect(
    screen.getByText(/does not match the selected Agent Key/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Gateway checks passed")).not.toBeInTheDocument();
  expect(document.body.textContent).not.toContain("synthetic-key-b");
  expect(localStorage.getItem(marker)).toBeNull();
});

it("attributes a late completion after navigation and account switch only to its initiator", async () => {
  let resolveAllowed!: (response: Response) => void;
  const pending = new Promise<Response>((resolve) => {
    resolveAllowed = resolve;
  });
  vi.spyOn(window, "fetch").mockImplementation(async (url) =>
    String(url).includes("__nyxid_test_denied__") ? gatewayDenial() : pending,
  );
  const user = userEvent.setup();
  const view = render(<VerifyKeyCard apiKey={key} />);
  await user.type(screen.getByLabelText("Agent key"), "nyxid_ag_test-secret");
  await user.click(screen.getByRole("button", { name: "Test key" }));
  view.unmount();
  useAuthStore.setState({
    user: { id: "user-b" } as User,
    isAuthenticated: true,
  });
  const { result } = renderHook(useProxyOnboarding);
  await act(async () => {
    resolveAllowed(proxied());
    await pending;
  });
  expect(localStorage.getItem(marker)).toBe("1");
  expect(
    localStorage.getItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-b`),
  ).toBeNull();
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  expect(result.current.verifyKeyLoading).toBe(false);
});
