import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectCardContentBlock } from "@/types/assistant";
import { ApiError } from "@/lib/api-client";
import { ConnectCard } from "./connect-card";

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  keys: [] as Array<Record<string, unknown>>,
  // The card resolves the real connect modality from the catalog rather than
  // trusting `block.auth_kind`. `null` entry + `isError: false` = still
  // loading (card falls back to the block's hint); `isError: true` = the slug
  // isn't in the catalog at all.
  catalogEntry: null as Record<string, unknown> | null,
  catalogError: null as unknown,
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: mocks.keys }),
  useCatalogEntry: () => ({
    data: mocks.catalogEntry ?? undefined,
    error: mocks.catalogError ?? null,
    isError: mocks.catalogError !== null,
  }),
}));

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    prefillSlug,
    reconnectKey,
  }: {
    readonly open: boolean;
    readonly prefillSlug?: string;
    readonly reconnectKey?: { readonly id: string } | null;
  }) =>
    open ? (
      <div
        data-testid="add-key-dialog"
        data-prefill={prefillSlug ?? ""}
        data-reconnect={reconnectKey?.id ?? ""}
      />
    ) : null,
}));

function blocker(
  reasonCode: NonNullable<ConnectCardContentBlock["reason_code"]>,
): ConnectCardContentBlock {
  return {
    type: "connect_card",
    block_id: "connect-1",
    catalog_slug: "api-github",
    service_name: "GitHub",
    icon_url: "",
    subtitle: "Required by this request",
    auth_kind: "oauth",
    requested_scopes: [],
    key_id: null,
    granted_scopes: null,
    device_user_code: null,
    device_verification_url: null,
    state: "needs_connection",
    error_message: null,
    steps: [
      {
        title: "Connect GitHub",
        body: "Connect or reauthorize GitHub to continue.",
        done: false,
      },
    ],
    footer: "Brokered by NyxID",
    reason_code: reasonCode,
  };
}

beforeEach(() => {
  mocks.keys = [];
  mocks.catalogEntry = null;
  mocks.catalogError = null;
  mocks.navigate.mockReset();
});

describe("ConnectCard authorization actions", () => {
  it("opens OAuth reauthorization for a matching unauthorized key", async () => {
    mocks.keys = [
      {
        id: "key-github",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    const user = userEvent.setup();
    render(<ConnectCard block={blocker("NYXID_UNAUTHORIZED")} />);

    expect(screen.getByText("Reauthorization required")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reconnect" }));

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "key-github",
    );
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

  it("prefers the card's exact key over an earlier slug match", async () => {
    mocks.keys = [
      {
        id: "key-same-slug",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        credential_type: "api_key",
        auth_method: "bearer",
      },
      {
        id: "key-exact",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    const user = userEvent.setup();
    render(
      <ConnectCard
        block={{ ...blocker("NYXID_UNAUTHORIZED"), key_id: "key-exact" }}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Reconnect" }));

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-reconnect",
      "key-exact",
    );
  });

  it("opens the canonical add-service flow for a disconnected service", async () => {
    mocks.catalogEntry = { slug: "api-github", name: "GitHub", provider_type: "oauth2" };
    const user = userEvent.setup();
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    await user.click(screen.getByRole("button", { name: "Connect" }));

    expect(screen.getByTestId("add-key-dialog")).toHaveAttribute(
      "data-prefill",
      "api-github",
    );
  });

  it("routes a matching non-OAuth credential to key management", async () => {
    mocks.keys = [
      {
        id: "key-api-token",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        credential_type: "api_key",
        auth_method: "bearer",
      },
    ];
    const user = userEvent.setup();
    render(<ConnectCard block={blocker("NYXID_UNAUTHORIZED")} />);

    await user.click(screen.getByRole("button", { name: "Manage" }));

    expect(mocks.navigate).toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "key-api-token" },
    });
  });
});

describe("ConnectCard catalog resolution", () => {
  it("keeps the action available when the catalog is merely unreachable", () => {
    // A 500 is not evidence the service is missing; removing the user's only
    // way forward on a transient failure is worse than a slightly wrong icon.
    mocks.catalogError = new ApiError(500, {
      error: "server_error",
      error_code: 500,
      message: "boom",
    });
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
    expect(screen.getByText(/Couldn't reach the NyxID catalog/i)).toBeInTheDocument();
  });

  it("hides the action while an authorization is already in flight", () => {
    // A second click would mint a second placeholder key for one service.
    mocks.catalogEntry = { slug: "api-github", name: "GitHub", provider_type: "oauth2" };
    mocks.keys = [
      {
        id: "key-github",
        catalog_service_slug: "api-github",
        is_active: false,
        auto_connected: false,
        status: "pending_auth",
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    expect(screen.queryByRole("button", { name: "Connect" })).not.toBeInTheDocument();
  });

  it("uses the catalog's modality and name, not the block's hint", () => {
    // The live authorization frame can't carry a modality, so the transport
    // fills in `api_key` as a placeholder. Trusting it would offer an OAuth
    // service a paste-your-key button.
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub (catalog)",
      provider_type: "oauth2",
    };
    const block = { ...blocker("NYXID_SERVICE_NOT_CONNECTED"), auth_kind: "api_key" as const };
    render(<ConnectCard block={block} />);

    expect(screen.getByText("GitHub (catalog)")).toBeInTheDocument();
    // OAuth affordance wins over the block's `api_key` hint.
    expect(
      screen.getByRole("button", { name: "Connect" }).querySelector("svg"),
    ).toHaveClass("lucide-external-link");
  });

  it("offers no connect action for a slug missing from the catalog", () => {
    mocks.catalogError = new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "no such service",
    });
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    expect(
      screen.queryByRole("button", { name: "Connect" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/isn't in your NyxID catalog/i),
    ).toBeInTheDocument();
  });

  it("streams authorization progress while the placeholder key is pending", () => {
    mocks.keys = [
      {
        id: "key-github",
        catalog_service_slug: "api-github",
        is_active: false,
        auto_connected: false,
        status: "pending_auth",
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    expect(screen.getByRole("status")).toHaveTextContent(/Waiting for GitHub/i);
    expect(screen.getByText("Authorizing")).toBeInTheDocument();
  });
});

describe("ConnectCard recovery paths", () => {
  it("keeps Manage reachable when the catalog 404s but the key still exists", () => {
    // Regression: the 404 guard hid every action, including one that needs no
    // catalog at all — stranding a key the user still owns.
    mocks.catalogError = new ApiError(404, {
      error: "not_found",
      error_code: 404,
      message: "no such service",
    });
    mocks.keys = [
      {
        id: "key-api-token",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        credential_type: "api_key",
        auth_method: "bearer",
      },
    ];
    render(<ConnectCard block={blocker("NYXID_UNAUTHORIZED")} />);

    expect(screen.getByRole("button", { name: "Manage" })).toBeInTheDocument();
  });
});
