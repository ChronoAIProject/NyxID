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

const { mockGet, mockPost, mockUseOrgs } = vi.hoisted(() => ({
  mockGet: vi.fn(),
  mockPost: vi.fn(),
  mockUseOrgs: vi.fn(),
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

// auth-flows pulls in OAuth / device-code subflows that aren't part of
// our path. Stub them out so they don't try to render.
vi.mock("./auth-flows", () => ({
  OAuthFlow: () => null,
  DeviceCodeFlow: () => null,
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
