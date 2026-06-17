import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

// Issue #414: AiKeyConfirm's custom-mode entry point and the
// CustomServiceForm's prefill consumption are the user-facing surface
// the new wizard delivers. These tests pin:
//   - `prefill.custom = true` skips the catalog grid entirely;
//   - prefill values pre-populate label / endpoint_url / auth_method
//     / auth_key_name / custom_slug;
//   - `prefill.via_node` shows the node-bound badge AND lands in the
//     POST /keys body so the backend pushes the credential downstream;
//   - omitting prefill.custom keeps the catalog grid as the entry
//     point — regression guard for the existing `service add <slug>`
//     flow.

const { mockGet, mockPost, mockUseOrgs, mockOAuthFlow, mockDeviceCodeFlow } =
  vi.hoisted(() => ({
    mockGet: vi.fn(),
    mockPost: vi.fn(),
    mockUseOrgs: vi.fn(),
    mockOAuthFlow: vi.fn(),
    mockDeviceCodeFlow: vi.fn(),
  }));

vi.mock("@/lib/api-client", () => ({
  api: {
    post: mockPost,
    get: mockGet,
  },
  ApiError: class ApiError extends Error {
    status: number;
    errorCode: number;
    constructor(
      status: number,
      response: { message: string; error_code: number },
    ) {
      super(response.message);
      this.status = status;
      this.errorCode = response.error_code;
    }
  },
}));

vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: mockUseOrgs,
}));

vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
    label,
  }: {
    readonly value: string | null;
    readonly onChange: (value: string | null) => void;
    readonly label?: string;
  }) => (
    <select
      aria-label={label ?? "Scope"}
      value={value ?? ""}
      onChange={(event) => onChange(event.target.value || null)}
    >
      <option value="">Personal</option>
      <option value="0a130a17-2624-4fbb-a69d-8ba51c99952a">ChronoAI</option>
    </select>
  ),
}));

// The wizard's "reserve action" + "rewind on error" helpers do their
// own backend calls to the cli-pairings endpoint. These are unrelated
// to the feature under test; stub them out.
vi.mock("@/pages/cli-pair/reserve-action", () => ({
  reservePairingAction: vi.fn().mockResolvedValue(undefined),
  withRewindOnError: vi.fn(async (_id: string, run: () => Promise<unknown>) => run()),
}));

// CatalogGrid hits its own endpoint that we don't care about for
// these tests. Render a marker so we can assert "the catalog grid
// is/isn't on screen."
vi.mock("./catalog-grid", () => ({
  CatalogGrid: () => <div data-testid="catalog-grid">CATALOG_GRID_MARKER</div>,
}));

// auth-flows pulls in OAuth / device-code subflows whose internals
// aren't under test here. Stub them with prop recorders so the
// additional-scopes handoff (issue #917) can be asserted without
// running the real placeholder/polling machinery.
vi.mock("./auth-flows", () => ({
  OAuthFlow: (props: Record<string, unknown>) => {
    mockOAuthFlow(props);
    return <div data-testid="oauth-flow">OAUTH_FLOW_MARKER</div>;
  },
  DeviceCodeFlow: (props: Record<string, unknown>) => {
    mockDeviceCodeFlow(props);
    return <div data-testid="device-code-flow">DEVICE_CODE_FLOW_MARKER</div>;
  },
}));

function createWrapper() {
  const queryClient = new QueryClient({
    defaultOptions: {
      mutations: { retry: false },
      queries: { retry: false },
    },
  });
  return function Wrapper({ children }: PropsWithChildren) {
    return (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
  };
}

import { AiKeyConfirm } from "./ai-key-confirm-panel";

const baseProps = {
  pairingId: "pair-test-123",
  onSuccess: vi.fn(),
  onSlugPicked: vi.fn(),
};

beforeEach(() => {
  mockUseOrgs.mockReturnValue({ data: [] });
});

function getFieldLabel(id: string) {
  const label = document.querySelector(`label[for="${id}"]`);
  if (!(label instanceof HTMLElement)) {
    throw new Error(`Missing label for ${id}`);
  }
  return label;
}

function expectRequiredMarkerFor(id: string) {
  const marker = getFieldLabel(id).parentElement?.querySelector(
    'span[aria-hidden="true"]',
  );
  if (!(marker instanceof HTMLElement)) {
    throw new Error(`Missing required marker for ${id}`);
  }
  expect(marker).toHaveTextContent("*");
  expect(marker).toBeVisible();
}

function expectNoRequiredMarkerFor(id: string) {
  expect(
    getFieldLabel(id).parentElement?.querySelector('span[aria-hidden="true"]'),
  ).toBeNull();
}

describe("AiKeyConfirm — issue #414 custom-mode entry", () => {
  beforeEach(() => {
    mockPost.mockReset();
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it("auto-opens the custom-service form when prefill.custom is true", () => {
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
          auth_key_name: "Authorization",
        }}
      />,
      { wrapper: createWrapper() },
    );

    // The form renders, NOT the catalog grid. This is the core
    // issue #414 promise: same command (`service add --custom`)
    // routes through the same wizard surface as `service add <slug>`,
    // but skips the catalog step the user can't use.
    expect(screen.queryByTestId("catalog-grid")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Label")).toBeInTheDocument();
    expect(screen.getByLabelText("Endpoint URL")).toBeInTheDocument();
    expect(screen.getByLabelText("Auth method")).toBeInTheDocument();
    // bearer doesn't need a key-name input (defaults to Authorization)
    expect(screen.queryByLabelText(/Header name/)).not.toBeInTheDocument();
  });

  it("renders the catalog grid when prefill.custom is unset", () => {
    // Regression guard: the existing `service add` (no flags) flow
    // still lands on the catalog grid as today.
    render(<AiKeyConfirm {...baseProps} prefill={{}} />, {
      wrapper: createWrapper(),
    });
    expect(screen.getByTestId("catalog-grid")).toBeInTheDocument();
  });

  it("pre-populates form fields from prefill in custom mode", () => {
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          label: "Home Assistant (Admin)",
          endpoint_url: "http://homeassistant.local:8123",
          custom_slug: "home-assistant-admin",
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("Label")).toHaveValue(
      "Home Assistant (Admin)",
    );
    expect(screen.getByLabelText("Endpoint URL")).toHaveValue(
      "http://homeassistant.local:8123",
    );
    expect(screen.getByLabelText("Custom slug (optional)")).toHaveValue(
      "home-assistant-admin",
    );
    expect(screen.getByLabelText("Auth method")).toHaveValue("bearer");
  });

  it("shows the node-bound badge when prefill.via_node is set", () => {
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          via_node: "node-uuid-12345",
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );
    expect(screen.getByText(/Routed via node/i)).toBeInTheDocument();
    expect(screen.getByText("node-uuid-12345")).toBeInTheDocument();
  });

  it("submits POST /keys with node_id and credential when via_node is set", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      id: "svc-id-abc",
      slug: "home-assistant-admin",
      label: "Home Assistant",
    });
    const onSuccess = vi.fn();

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          via_node: "node-uuid-12345",
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
          auth_key_name: "Authorization",
        }}
        onSuccess={onSuccess}
      />,
      { wrapper: createWrapper() },
    );

    // Paste the credential and submit. label / endpoint / auth_method
    // / auth_key_name come from prefill; node_id from prefill.via_node.
    await user.type(
      screen.getByLabelText("API key / credential"),
      "tok_abc123",
    );
    await user.click(screen.getByRole("button", { name: /Connect service/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith("/keys", {
        label: "Home Assistant",
        endpoint_url: "http://homeassistant.local:8123",
        auth_method: "bearer",
        credential: "tok_abc123",
        node_id: "node-uuid-12345",
      });
    });

    // onSuccess fires with the typed `ai-key` shape — same as the
    // catalog flow uses, so the CLI's existing printer handles it.
    await waitFor(() => {
      expect(onSuccess).toHaveBeenCalledWith({
        kind: "ai-key",
        service_id: "svc-id-abc",
        slug: "home-assistant-admin",
        label: "Home Assistant",
      });
    });
  });

  it("includes auth_key_name in the body for header / query / path / body methods", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      id: "svc-id-abc",
      slug: "x",
      label: "x",
    });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          label: "X",
          endpoint_url: "http://x.local",
          auth_method: "header",
          auth_key_name: "X-Custom-Key",
        }}
      />,
      { wrapper: createWrapper() },
    );

    // The X-Custom-Key from prefill should land in the form's
    // "Header name" field.
    expect(screen.getByLabelText("Header name")).toHaveValue("X-Custom-Key");

    await user.type(
      screen.getByLabelText("API key / credential"),
      "secret-value",
    );
    await user.click(screen.getByRole("button", { name: /Connect service/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/keys",
        expect.objectContaining({
          auth_method: "header",
          auth_key_name: "X-Custom-Key",
          credential: "secret-value",
        }),
      );
    });
  });

  // Regression guard: issue #414 widened the wizard's auth-method
  // dropdown to cover all 8 methods the backend accepts. Each
  // method is its own test case (rather than a loop in one test)
  // so a single regression flags one specific value rather than
  // failing the whole batch.
  it.each(["bearer", "bot_bearer", "header", "query", "path", "basic", "body", "none"] as const)(
    "supports auth_method=%s in the dropdown",
    (method) => {
      render(
        <AiKeyConfirm
          {...baseProps}
          prefill={{
            custom: true,
            label: "X",
            endpoint_url: "http://x.local",
            auth_method: method,
          }}
        />,
        { wrapper: createWrapper() },
      );
      expect(screen.getByLabelText("Auth method")).toHaveValue(method);
    },
  );
});

describe("AiKeyConfirm — org-scoped owner picker", () => {
  beforeEach(() => {
    mockPost.mockReset();
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it("hides the owner picker when the user has no admin orgs", () => {
    mockUseOrgs.mockReturnValue({
      data: [
        {
          id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
          display_name: "ChronoAI",
          your_role: "member",
        },
      ],
    });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.queryByLabelText("Owner")).not.toBeInTheDocument();
  });

  it("includes target_org_id in POST /keys when an org is selected", async () => {
    const user = userEvent.setup();
    mockUseOrgs.mockReturnValue({
      data: [
        {
          id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
          display_name: "ChronoAI",
          your_role: "admin",
        },
      ],
    });
    mockPost.mockResolvedValue({
      id: "svc-id-abc",
      slug: "home-assistant-admin",
      label: "Home Assistant",
    });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    await user.selectOptions(
      screen.getByLabelText("Owner"),
      "0a130a17-2624-4fbb-a69d-8ba51c99952a",
    );
    await user.type(
      screen.getByLabelText("API key / credential"),
      "tok_abc123",
    );
    await user.click(screen.getByRole("button", { name: /Connect service/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/keys",
        expect.objectContaining({
          target_org_id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
        }),
      );
    });
  });

  it("pre-selects the owner from prefill org_id", () => {
    mockUseOrgs.mockReturnValue({
      data: [
        {
          id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
          display_name: "ChronoAI",
          your_role: "admin",
        },
      ],
    });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          org_id: "0a130a17-2624-4fbb-a69d-8ba51c99952a",
          label: "Home Assistant",
          endpoint_url: "http://homeassistant.local:8123",
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("Owner")).toHaveValue(
      "0a130a17-2624-4fbb-a69d-8ba51c99952a",
    );
  });
});

describe("AiKeyConfirm — custom-service required markers", () => {
  beforeEach(() => {
    mockPost.mockReset();
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it("marks required fields in the custom-service form", () => {
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    expectRequiredMarkerFor("pair-custom-label");
    expectRequiredMarkerFor("pair-custom-endpoint");
    expectRequiredMarkerFor("pair-custom-auth-method");
    expectRequiredMarkerFor("pair-custom-credential");
    expectNoRequiredMarkerFor("pair-custom-slug");

    expect(screen.getByLabelText("Label")).toHaveAttribute(
      "aria-required",
      "true",
    );
    expect(screen.getByLabelText("Endpoint URL")).toHaveAttribute(
      "aria-required",
      "true",
    );
    expect(screen.getByLabelText("Auth method")).toHaveAttribute(
      "aria-required",
      "true",
    );
    expect(screen.getByLabelText("API key / credential")).toHaveAttribute(
      "aria-required",
      "true",
    );
    expect(getFieldLabel("pair-custom-slug")).not.toHaveTextContent("*");
  });

  it("removes the credential field when auth method is none", async () => {
    const user = userEvent.setup();

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          custom: true,
          auth_method: "bearer",
        }}
      />,
      { wrapper: createWrapper() },
    );

    expect(screen.getByLabelText("API key / credential")).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("Auth method"), "none");

    expect(
      screen.queryByLabelText("API key / credential"),
    ).not.toBeInTheDocument();
  });
});

describe("AiKeyConfirm — custom-service back reset", () => {
  beforeEach(() => {
    mockPost.mockReset();
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it('notifies with "" when Back resets the custom-service slug', async () => {
    const user = userEvent.setup();
    const onSlugPicked = vi.fn();

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{ custom: true }}
        onSlugPicked={onSlugPicked}
      />,
      { wrapper: createWrapper() },
    );

    await waitFor(() => {
      expect(onSlugPicked).toHaveBeenCalledWith("__custom__");
    });
    onSlugPicked.mockClear();

    await user.click(screen.getByRole("button", { name: /Back/i }));

    await waitFor(() => {
      expect(onSlugPicked).toHaveBeenLastCalledWith("");
    });
    expect(onSlugPicked).toHaveBeenCalledTimes(1);
  });
});

describe("AiKeyConfirm — catalog services routed via node", () => {
  const catalogEntry = {
    slug: "llm-openai",
    name: "OpenAI",
    base_url: "https://api.openai.com",
    auth_method: "bearer",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
  };

  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockGet.mockResolvedValue(catalogEntry);
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it("hides credential, shows node badge, and omits credential in POST body", async () => {
    const user = userEvent.setup();
    mockPost.mockResolvedValue({
      id: "svc-id-abc",
      slug: "llm-openai",
      label: "Test",
    });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          slug: "llm-openai",
          via_node: "node-uuid-12345",
          label: "Test",
        }}
      />,
      { wrapper: createWrapper() },
    );

    await screen.findByText(/Routed via node/i);

    expect(screen.queryByLabelText(/API key/i)).not.toBeInTheDocument();
    expect(screen.getByText("node-uuid-12345")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Connect via node/i }));

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalledWith(
        "/keys",
        expect.objectContaining({
          service_slug: "llm-openai",
          label: "Test",
          node_id: "node-uuid-12345",
        }),
      );
    });

    const [, body] = mockPost.mock.calls.find(([path]) => path === "/keys") ?? [];
    expect(body).not.toHaveProperty("credential");
  });

  it("enables submit without a credential when via_node is set", async () => {
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          slug: "llm-openai",
          via_node: "node-uuid-12345",
          label: "Test",
        }}
      />,
      { wrapper: createWrapper() },
    );

    await screen.findByText(/Routed via node/i);

    expect(
      screen.getByRole("button", { name: /Connect via node/i }),
    ).not.toBeDisabled();
  });

  it("keeps submit disabled when label is empty", async () => {
    const user = userEvent.setup();
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          slug: "llm-openai",
          via_node: "node-uuid-12345",
          label: "Test",
        }}
      />,
      { wrapper: createWrapper() },
    );

    await screen.findByText(/Routed via node/i);

    await user.clear(screen.getByLabelText("Label"));

    expect(
      screen.getByRole("button", { name: /Connect via node/i }),
    ).toBeDisabled();
  });
});

// NyxID#917 follow-up — the pair wizard renders the shared scope picker for
// OAuth / device-code providers (defaults pre-selected as pills + a custom
// "Add" field), gated identically (`device_code_format !== "openai"`), and
// hands the COMPLETE selection to the auth sub-flow as `scopeOverride`.
describe("AiKeyConfirm — upstream scope picker (issue #917)", () => {
  const oauthEntry = {
    slug: "social-twitter",
    name: "Twitter / X",
    base_url: "https://api.twitter.com",
    auth_method: "bearer",
    provider_type: "oauth2",
    provider_config_id: "prov-twitter",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
    default_scopes: ["tweet.read", "users.read"],
    scope_catalog: [
      { scope: "tweet.read", label: "Read posts", description: "Read posts." },
      { scope: "media.write", label: "Upload media", description: "Upload media.", sensitive: true },
    ],
  };
  const deviceCodeEntry = {
    slug: "vcs-github",
    name: "GitHub",
    base_url: "https://api.github.com",
    auth_method: "bearer",
    provider_type: "device_code",
    provider_config_id: "prov-github",
    device_code_format: "oauth2",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
    default_scopes: ["read:user"],
    scope_catalog: [
      { scope: "read:user", label: "Read profile", description: "Read profile." },
      { scope: "repo", label: "Repositories", description: "Full repo access.", sensitive: true },
    ],
  };
  const openaiDeviceCodeEntry = {
    ...deviceCodeEntry,
    slug: "llm-codex",
    name: "Codex",
    provider_config_id: "prov-codex",
    device_code_format: "openai",
    scope_catalog: null,
  };
  const apiKeyEntry = {
    slug: "llm-openai",
    name: "OpenAI",
    base_url: "https://api.openai.com",
    auth_method: "bearer",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
  };

  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockOAuthFlow.mockReset();
    mockDeviceCodeFlow.mockReset();
    baseProps.onSuccess = vi.fn();
    baseProps.onSlugPicked = vi.fn();
  });

  it("pre-selects defaults and forwards the picker selection (incl. a toggled-on scope) to OAuth", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue(oauthEntry);

    render(
      <AiKeyConfirm {...baseProps} prefill={{ slug: "social-twitter" }} />,
      { wrapper: createWrapper() },
    );

    // Defaults render as pills; "Upload media" (media.write) starts off.
    const mediaPill = await screen.findByRole("button", {
      name: /Upload media/i,
    });
    await user.click(mediaPill); // toggle it on
    await user.click(
      screen.getByRole("button", { name: /Continue with provider sign-in/i }),
    );

    await waitFor(() => {
      expect(mockOAuthFlow).toHaveBeenCalled();
    });
    const props = mockOAuthFlow.mock.calls.at(-1)?.[0] as {
      readonly providerId: string;
      readonly scopeOverride: readonly string[];
    };
    expect(props.providerId).toBe("prov-twitter");
    // Defaults stay selected; the toggled-on catalog scope is added.
    expect([...props.scopeOverride].sort()).toEqual(
      ["media.write", "tweet.read", "users.read"].sort(),
    );
  });

  it("adds a custom scope via the Add field and forwards it to OAuth", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue(oauthEntry);

    render(
      <AiKeyConfirm {...baseProps} prefill={{ slug: "social-twitter" }} />,
      { wrapper: createWrapper() },
    );

    const custom = await screen.findByPlaceholderText(/media\.write/i);
    await user.type(custom, "dm.read");
    await user.click(screen.getByRole("button", { name: /^Add$/i }));
    await user.click(
      screen.getByRole("button", { name: /Continue with provider sign-in/i }),
    );

    await waitFor(() => {
      expect(mockOAuthFlow).toHaveBeenCalled();
    });
    const props = mockOAuthFlow.mock.calls.at(-1)?.[0] as {
      readonly scopeOverride: readonly string[];
    };
    expect(props.scopeOverride).toContain("dm.read");
    expect(props.scopeOverride).toContain("tweet.read");
  });

  it("forwards the picker selection to the device-code sub-flow", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue(deviceCodeEntry);

    render(<AiKeyConfirm {...baseProps} prefill={{ slug: "vcs-github" }} />, {
      wrapper: createWrapper(),
    });

    await screen.findByRole("button", { name: /Repositories/i });
    await user.click(screen.getByRole("button", { name: /Get device code/i }));

    await waitFor(() => {
      expect(mockDeviceCodeFlow).toHaveBeenCalled();
    });
    const props = mockDeviceCodeFlow.mock.calls.at(-1)?.[0] as {
      readonly providerId: string;
      readonly scopeOverride: readonly string[];
    };
    expect(props.providerId).toBe("prov-github");
    expect(props.scopeOverride).toEqual(["read:user"]); // default pre-selected
  });

  it("hides the picker for openai-format device-code providers and passes no override", async () => {
    const user = userEvent.setup();
    mockGet.mockResolvedValue(openaiDeviceCodeEntry);

    render(<AiKeyConfirm {...baseProps} prefill={{ slug: "llm-codex" }} />, {
      wrapper: createWrapper(),
    });

    await screen.findByRole("button", { name: /Get device code/i });
    expect(screen.queryByRole("button", { name: /^Add$/i })).not.toBeInTheDocument();
    // Parity with the dashboard: explain WHY there's no picker.
    expect(
      screen.getByText(/does not accept additional scopes/i),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /Get device code/i }));

    await waitFor(() => {
      expect(mockDeviceCodeFlow).toHaveBeenCalled();
    });
    const props = mockDeviceCodeFlow.mock.calls.at(-1)?.[0] as {
      readonly scopeOverride: readonly string[] | undefined;
    };
    expect(props.scopeOverride).toBeUndefined();
  });

  it("does not render the scope picker for plain api-key providers", async () => {
    mockGet.mockResolvedValue(apiKeyEntry);

    render(<AiKeyConfirm {...baseProps} prefill={{ slug: "llm-openai" }} />, {
      wrapper: createWrapper(),
    });

    await screen.findByLabelText(/API key/i);
    expect(screen.queryByRole("button", { name: /^Add$/i })).not.toBeInTheDocument();
    expect(
      screen.queryByText(/does not accept additional scopes/i),
    ).not.toBeInTheDocument();
  });
});

describe("AiKeyConfirm — manage-scopes (NyxID#917 codex follow-up)", () => {
  // Codex's tech-lead consult: assert that when an existing connection has
  // no recorded grant (granted_scopes=[]) — common for legacy keys or
  // providers that didn't return `scope` in their token response — the
  // wizard sends `scopeOverride: undefined` so the backend uses catalog
  // defaults, instead of sending [] which the backend treats as
  // "drop all defaults" → silent zero-scope re-auth.
  const oauthCatalogEntry = {
    slug: "social-twitter",
    name: "Twitter / X",
    base_url: "https://api.twitter.com",
    auth_method: "bearer",
    provider_type: "oauth2",
    provider_config_id: "prov-twitter",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
    default_scopes: ["tweet.read", "users.read"],
    scope_catalog: [
      { scope: "tweet.read", label: "Read posts", description: "Read posts." },
      { scope: "users.read", label: "Read profile", description: "Read profile." },
      { scope: "media.write", label: "Upload media", description: "Upload media.", sensitive: true },
    ],
  };

  function mockManageScopesEndpoints(opts: {
    grantedScopes: readonly string[] | null;
  }) {
    mockGet.mockImplementation((url: string) => {
      if (url.startsWith("/keys/")) {
        return Promise.resolve({
          id: "key-legacy-abc",
          label: "My Twitter",
          slug: "social-twitter",
          catalog_service_slug: "social-twitter",
          credential_type: "oauth2",
          granted_scopes: opts.grantedScopes,
          last_authorized_at: null,
        });
      }
      if (url.startsWith("/catalog/")) {
        return Promise.resolve(oauthCatalogEntry);
      }
      return Promise.reject(new Error(`unexpected GET ${url}`));
    });
  }

  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockOAuthFlow.mockReset();
    baseProps.onSuccess = vi.fn();
  });

  it("sends scopeOverride=undefined on re-auth when the existing connection has no recorded grant", async () => {
    // Setup: legacy key with granted_scopes = [] (no recorded grant). The
    // picker seeds from catalog defaults for display, but until the user
    // edits, no override is transmitted.
    const user = userEvent.setup();
    mockManageScopesEndpoints({ grantedScopes: [] });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{ reconnect_key_id: "key-legacy-abc" }}
      />,
      { wrapper: createWrapper() },
    );

    // Wait for the ManageScopesPanel to mount.
    await screen.findByRole("heading", { name: /Manage permissions/i });
    await user.click(
      screen.getByRole("button", { name: /Re-authorize with these permissions/i }),
    );

    await waitFor(() => {
      expect(mockOAuthFlow).toHaveBeenCalled();
    });
    const props = mockOAuthFlow.mock.calls.at(-1)?.[0] as {
      readonly scopeOverride: readonly string[] | undefined;
      readonly reconnectKeyId: string;
    };
    expect(props.reconnectKeyId).toBe("key-legacy-abc");
    expect(props.scopeOverride).toBeUndefined();
  });

  it("forwards an explicit empty array when the user actively clears all picker scopes", async () => {
    // Setup: existing connection HAS a recorded grant. User deliberately
    // toggles every default pill OFF (intent to revoke all scopes). The
    // wizard must respect that — scopeOverride: [] is the user's stated
    // intent here, distinct from the legacy-empty-grant case above.
    const user = userEvent.setup();
    mockManageScopesEndpoints({ grantedScopes: ["tweet.read", "users.read"] });

    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{ reconnect_key_id: "key-legacy-abc" }}
      />,
      { wrapper: createWrapper() },
    );

    await screen.findByRole("heading", { name: /Manage permissions/i });
    // Toggle off both granted defaults.
    await user.click(await screen.findByRole("button", { name: /Read posts/i }));
    await user.click(await screen.findByRole("button", { name: /Read profile/i }));
    await user.click(
      screen.getByRole("button", { name: /Re-authorize with these permissions/i }),
    );

    await waitFor(() => {
      expect(mockOAuthFlow).toHaveBeenCalled();
    });
    const props = mockOAuthFlow.mock.calls.at(-1)?.[0] as {
      readonly scopeOverride: readonly string[] | undefined;
    };
    expect(props.scopeOverride).toBeDefined();
    expect([...(props.scopeOverride ?? [])]).toEqual([]);
  });
});

// NyxID#917 follow-up — manage-scopes mode: `nyxid service scopes <slug> --set`
// sends prefill.reconnect_key_id + prefill.scope_override. The panel must fetch
// the connection, render the picker seeded with EXACTLY the --set scopes, and
// hand them to OAuthFlow as scopeOverride + reconnectKeyId.
describe("AiKeyConfirm — manage-scopes mode (issue #917 CLI --set)", () => {
  const twitterEntry = {
    slug: "api-twitter",
    name: "Twitter / X API",
    base_url: "https://api.twitter.com",
    auth_method: "bearer",
    provider_type: "oauth2",
    provider_config_id: "prov-twitter",
    credential_mode: "user",
    service_type: "rest",
    requires_credential: true,
    requires_gateway_url: false,
    default_scopes: ["tweet.read", "tweet.write", "users.read", "offline.access"],
    scope_catalog: [
      { scope: "tweet.read", label: "Read posts", description: "Read posts." },
      { scope: "media.write", label: "Upload media", description: "Upload media.", sensitive: true },
    ],
    scope_removal: "auto",
  };
  const existingKey = {
    id: "svc-1",
    slug: "api-twitter",
    label: "X Demo",
    status: "active",
    catalog_service_slug: "api-twitter",
    granted_scopes: ["tweet.read", "tweet.write", "users.read", "offline.access"],
    last_authorized_at: "2026-06-16T00:00:00Z",
  };

  beforeEach(() => {
    mockGet.mockReset();
    mockPost.mockReset();
    mockOAuthFlow.mockReset();
    mockGet.mockImplementation(async (path: string) => {
      if (path === "/keys/svc-1") return existingKey;
      if (path === "/catalog/api-twitter") return twitterEntry;
      throw new Error(`unexpected GET ${path}`);
    });
    baseProps.onSuccess = vi.fn();
  });

  it("seeds the picker from --set scope_override and hands it to OAuthFlow", async () => {
    const user = userEvent.setup();
    render(
      <AiKeyConfirm
        {...baseProps}
        prefill={{
          reconnect_key_id: "svc-1",
          scope_override: ["tweet.read", "media.write"],
        }}
      />,
      { wrapper: createWrapper() },
    );

    const reauth = await screen.findByRole("button", {
      name: /Re-authorize with these permissions/i,
    });
    // Exactly the --set scopes are pre-selected — NOT the connection's 4
    // current scopes.
    expect(
      screen.getByRole("button", { name: /Read posts/i }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /Upload media/i }),
    ).toHaveAttribute("aria-pressed", "true");

    await user.click(reauth);
    expect(mockOAuthFlow).toHaveBeenCalledWith(
      expect.objectContaining({
        providerId: "prov-twitter",
        reconnectKeyId: "svc-1",
        scopeOverride: ["tweet.read", "media.write"],
        baselineAuthorizedAt: "2026-06-16T00:00:00Z",
      }),
    );
  });

  it("without --set, seeds from the connection's current grant", async () => {
    render(
      <AiKeyConfirm {...baseProps} prefill={{ reconnect_key_id: "svc-1" }} />,
      { wrapper: createWrapper() },
    );
    await screen.findByRole("button", {
      name: /Re-authorize with these permissions/i,
    });
    // Connection's granted "Read posts" is selected; "Upload media" (not
    // granted) is not.
    expect(
      screen.getByRole("button", { name: /Read posts/i }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /Upload media/i }),
    ).toHaveAttribute("aria-pressed", "false");
  });
});
