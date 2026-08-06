import { act, render, screen, waitFor } from "@testing-library/react";
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
  addDialogProps: null as Record<string, unknown> | null,
  invalidateQueries: vi.fn(),
  watch: {
    status: undefined as string | undefined,
    authorized: false,
    errorMessage: undefined as string | undefined,
    timedOut: false,
  },
}));

vi.mock("@tanstack/react-query", async () => {
  const actual = await vi.importActual<typeof import("@tanstack/react-query")>(
    "@tanstack/react-query",
  );
  return {
    ...actual,
    useQueryClient: () => ({ invalidateQueries: mocks.invalidateQueries }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-keys", () => ({
  KEY_AUTH_FAILED: "failed",
  useKeys: () => ({ data: mocks.keys }),
  useKeyAuthorizationWatch: () => mocks.watch,
  useCatalogEntry: () => ({
    data: mocks.catalogEntry ?? undefined,
    error: mocks.catalogError ?? null,
    isError: mocks.catalogError !== null,
  }),
}));

vi.mock("@/hooks/use-chat-presence", () => ({
  useChatPresence: () => ({ visible: true, lastActivityAt: 0 }),
}));

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    prefillSlug,
    reconnectKey,
    ...props
  }: {
    readonly open: boolean;
    readonly prefillSlug?: string;
    readonly reconnectKey?: { readonly id: string } | null;
    readonly launch?: "popup";
    readonly flow?: "cc";
    readonly onPopupViewResult?: (keyId: string) => boolean;
    readonly onAuthorizationPending?: (attempt: {
      readonly keyId: string;
      readonly attemptId: string;
      readonly previousAuthorizationAt: string | null | undefined;
    }) => void;
    readonly onAuthorizationAborted?: (attemptId: string) => void;
  }) => (
    (mocks.addDialogProps = { open, prefillSlug, reconnectKey, ...props }),
    open ? (
      <div
        data-testid="add-key-dialog"
        data-prefill={prefillSlug ?? ""}
        data-reconnect={reconnectKey?.id ?? ""}
      />
    ) : null
  ),
}));

vi.mock("@/components/assistant/manage-connection-modal", () => ({
  ManageConnectionModal: ({
    keyIds,
  }: {
    readonly keyIds: readonly string[];
  }) => <div data-testid="manage-connection-modal" data-key-id={keyIds[0]} />,
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
  mocks.addDialogProps = null;
  mocks.invalidateQueries.mockReset();
  mocks.watch = {
    status: undefined,
    authorized: false,
    errorMessage: undefined,
    timedOut: false,
  };
  mocks.navigate.mockReset();
});

describe("ConnectCard authorization actions", () => {
  it("opts chat OAuth into popup mode and replaces the add dialog for result view", async () => {
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
    const user = userEvent.setup();
    render(<ConnectCard block={blocker("NYXID_SERVICE_NOT_CONNECTED")} />);

    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect(screen.getByTestId("add-key-dialog")).toBeInTheDocument();
    expect(mocks.addDialogProps).toMatchObject({ launch: "popup", flow: "cc" });
    const onPopupViewResult = mocks.addDialogProps?.onPopupViewResult as
      | ((keyId: string) => boolean)
      | undefined;
    expect(onPopupViewResult).toBeTypeOf("function");

    let handled = false;
    act(() => {
      handled = onPopupViewResult?.("key-popup") ?? false;
    });
    expect(handled).toBe(true);
    expect(screen.queryByTestId("add-key-dialog")).not.toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("manage-connection-modal")).toHaveAttribute(
        "data-key-id",
        "key-popup",
      ),
    );
    expect(mocks.navigate).not.toHaveBeenCalled();
  });

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
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
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

describe("ConnectCard authorization settlement", () => {
  it("shows authorizing immediately and settles active, failed, and timed-out attempts", async () => {
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
    const user = userEvent.setup();
    const block = blocker("NYXID_SERVICE_NOT_CONNECTED");
    const { rerender } = render(<ConnectCard block={block} />);
    await user.click(screen.getByRole("button", { name: "Connect" }));
    const pending = mocks.addDialogProps?.onAuthorizationPending as
      | ((attempt: {
          keyId: string;
          attemptId: string;
          previousAuthorizationAt: undefined;
        }) => void)
      | undefined;

    act(() =>
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-active",
        previousAuthorizationAt: undefined,
      }),
    );
    expect(screen.getByText("Authorizing")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Connect" }),
    ).not.toBeInTheDocument();

    mocks.watch = {
      status: "active",
      authorized: true,
      errorMessage: undefined,
      timedOut: false,
    };
    rerender(<ConnectCard block={block} />);
    expect(await screen.findByText("Connected")).toBeInTheDocument();

    mocks.watch = {
      status: undefined,
      authorized: false,
      errorMessage: undefined,
      timedOut: false,
    };
    act(() =>
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-failed",
        previousAuthorizationAt: undefined,
      }),
    );
    expect(screen.getByText("Authorizing")).toBeInTheDocument();
    mocks.watch = {
      status: "failed",
      authorized: false,
      errorMessage: "Authorization was declined",
      timedOut: false,
    };
    rerender(<ConnectCard block={block} />);
    expect(await screen.findByText("Failed")).toBeInTheDocument();
    expect(screen.getByText("Authorization was declined")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();

    mocks.watch = {
      status: undefined,
      authorized: false,
      errorMessage: undefined,
      timedOut: false,
    };
    act(() =>
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-timeout",
        previousAuthorizationAt: undefined,
      }),
    );
    expect(screen.getByText("Authorizing")).toBeInTheDocument();
    mocks.watch = {
      status: "pending_auth",
      authorized: false,
      errorMessage: undefined,
      timedOut: true,
    };
    rerender(<ConnectCard block={block} />);
    expect(await screen.findByText("Timed out")).toBeInTheDocument();
  });

  it("keeps reconnect authorizing until its baseline advances", async () => {
    mocks.keys = [
      {
        id: "key-github",
        catalog_service_slug: "api-github",
        is_active: true,
        status: "active",
        last_authorized_at: "2026-08-06T10:00:00Z",
        auto_connected: false,
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    const user = userEvent.setup();
    const block = blocker("NYXID_UNAUTHORIZED");
    const { rerender } = render(<ConnectCard block={block} />);
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    const pending = mocks.addDialogProps?.onAuthorizationPending as
      | ((attempt: {
          keyId: string;
          attemptId: string;
          previousAuthorizationAt: string;
        }) => void)
      | undefined;
    act(() =>
      pending?.({
        keyId: "key-github",
        attemptId: "attempt-reconnect",
        previousAuthorizationAt: "2026-08-06T10:00:00Z",
      }),
    );

    mocks.watch = {
      status: "active",
      authorized: false,
      errorMessage: undefined,
      timedOut: false,
    };
    rerender(<ConnectCard block={block} />);
    expect(screen.getByText("Authorizing")).toBeInTheDocument();

    mocks.watch = { ...mocks.watch, authorized: true };
    rerender(<ConnectCard block={block} />);
    expect(await screen.findByText("Connected")).toBeInTheDocument();
    expect(
      screen.queryByText("Reauthorization required"),
    ).not.toBeInTheDocument();
  });

  it("renders a matching abort as neutral cancellation and ignores stale aborts", async () => {
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
    const user = userEvent.setup();
    const block = blocker("NYXID_SERVICE_NOT_CONNECTED");
    const { rerender } = render(<ConnectCard block={block} />);
    await user.click(screen.getByRole("button", { name: "Connect" }));
    const pending = mocks.addDialogProps?.onAuthorizationPending as
      | ((attempt: {
          keyId: string;
          attemptId: string;
          previousAuthorizationAt: undefined;
        }) => void)
      | undefined;
    act(() => {
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-a",
        previousAuthorizationAt: undefined,
      });
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-b",
        previousAuthorizationAt: undefined,
      });
    });
    mocks.keys = [
      {
        id: "key-1",
        catalog_service_slug: "api-github",
        is_active: false,
        auto_connected: false,
        status: "pending_auth",
        credential_type: "oauth2",
        auth_method: "oauth2",
      },
    ];
    rerender(<ConnectCard block={block} />);
    const aborted = mocks.addDialogProps?.onAuthorizationAborted as
      | ((attemptId: string) => void)
      | undefined;
    act(() => aborted?.("attempt-a"));
    expect(screen.getByText("Authorizing")).toBeInTheDocument();

    act(() => aborted?.("attempt-b"));
    expect(screen.getByText(/Connection cancelled/i)).toBeInTheDocument();
    expect(screen.queryByText("Failed")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
    expect(mocks.invalidateQueries).toHaveBeenCalledTimes(2);
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["keys"],
      exact: true,
    });
    expect(mocks.invalidateQueries).toHaveBeenCalledWith({
      queryKey: ["keys", "key-1"],
      exact: true,
    });
  });

  it("lets an advanced reconnect authorization override a simultaneous cancellation", async () => {
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
    mocks.keys = [
      {
        id: "key-1",
        catalog_service_slug: "api-github",
        is_active: true,
        auto_connected: false,
        status: "active",
        credential_type: "oauth2",
        auth_method: "oauth2",
        last_authorized_at: "2026-08-06T10:00:00Z",
      },
    ];
    const user = userEvent.setup();
    const block = blocker("NYXID_UNAUTHORIZED");
    const { rerender } = render(<ConnectCard block={block} />);
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    const pending = mocks.addDialogProps?.onAuthorizationPending as
      | ((attempt: {
          keyId: string;
          attemptId: string;
          previousAuthorizationAt: string;
        }) => void)
      | undefined;
    act(() =>
      pending?.({
        keyId: "key-1",
        attemptId: "attempt-reconnect",
        previousAuthorizationAt: "2026-08-06T10:00:00Z",
      }),
    );
    const aborted = mocks.addDialogProps?.onAuthorizationAborted as
      | ((attemptId: string) => void)
      | undefined;
    act(() => aborted?.("attempt-reconnect"));
    expect(screen.getByText(/Connection cancelled/i)).toBeInTheDocument();

    mocks.keys = [
      {
        ...mocks.keys[0],
        last_authorized_at: "2026-08-06T10:05:00Z",
      },
    ];
    rerender(<ConnectCard block={block} />);

    expect(screen.getByText("Connected")).toBeInTheDocument();
    expect(screen.queryByText(/Connection cancelled/i)).not.toBeInTheDocument();
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
    expect(
      screen.getByText(/Couldn't reach the NyxID catalog/i),
    ).toBeInTheDocument();
  });

  it("hides the action while an authorization is already in flight", () => {
    // A second click would mint a second placeholder key for one service.
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "oauth2",
    };
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

    expect(
      screen.queryByRole("button", { name: "Connect" }),
    ).not.toBeInTheDocument();
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
    const block = {
      ...blocker("NYXID_SERVICE_NOT_CONNECTED"),
      auth_kind: "api_key" as const,
    };
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

describe("ConnectCard acknowledges the full block contract", () => {
  // Basic, deliberately unstyled rendering. The point is that no §3.5 field is
  // silently dropped — the visual pass comes later.
  it("shows the device code and verification URL", () => {
    mocks.catalogEntry = {
      slug: "api-github",
      name: "GitHub",
      provider_type: "device_code",
    };
    render(
      <ConnectCard
        block={{
          ...blocker("NYXID_SERVICE_NOT_CONNECTED"),
          device_user_code: "WDJB-MJHT",
          device_verification_url: "https://github.com/login/device",
        }}
      />,
    );

    expect(screen.getByText("WDJB-MJHT")).toBeInTheDocument();
    expect(
      screen.getByRole("link", { name: /github\.com\/login\/device/ }),
    ).toHaveAttribute("href", "https://github.com/login/device");
  });

  it("lists a multi-step wizard rather than only its first line", () => {
    render(
      <ConnectCard
        block={{
          ...blocker("NYXID_SERVICE_NOT_CONNECTED"),
          steps: [
            { title: "Authorize NyxID", body: "Approve access.", done: true },
            { title: "NyxID seals the credential", body: "", done: false },
            { title: "Task resumes", body: "", done: false },
          ],
        }}
      />,
    );

    expect(screen.getByText(/NyxID seals the credential/)).toBeInTheDocument();
    expect(screen.getByText(/Task resumes/)).toBeInTheDocument();
  });

  it("shows requested scopes, and granted ones once they exist", () => {
    const base = blocker("NYXID_SERVICE_NOT_CONNECTED");
    const { rerender } = render(
      <ConnectCard block={{ ...base, requested_scopes: ["repo"] }} />,
    );
    expect(screen.getByText(/Requests: repo/)).toBeInTheDocument();

    rerender(
      <ConnectCard
        block={{
          ...base,
          requested_scopes: ["repo"],
          granted_scopes: ["repo", "read:user"],
        }}
      />,
    );
    expect(screen.getByText(/Granted: repo, read:user/)).toBeInTheDocument();
  });

  it("renders the broker footer", () => {
    render(
      <ConnectCard
        block={{
          ...blocker("NYXID_SERVICE_NOT_CONNECTED"),
          footer: "Brokered by NyxID · revoke anytime",
        }}
      />,
    );

    expect(screen.getByText(/Brokered by NyxID/)).toBeInTheDocument();
  });

  it("stays compact when the block carries no extra detail", () => {
    render(
      <ConnectCard
        block={{
          ...blocker("NYXID_SERVICE_NOT_CONNECTED"),
          footer: "",
          requested_scopes: [],
        }}
      />,
    );

    expect(screen.queryByText(/Requests:/)).not.toBeInTheDocument();
  });
});
