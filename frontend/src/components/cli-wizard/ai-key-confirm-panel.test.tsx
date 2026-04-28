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

const { mockPost } = vi.hoisted(() => ({
  mockPost: vi.fn(),
}));

vi.mock("@/lib/api-client", () => ({
  api: {
    post: mockPost,
    get: vi.fn(),
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
