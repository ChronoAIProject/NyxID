import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectCardContentBlock } from "@/types/assistant";
import { ConnectCard } from "./connect-card";

const mocks = vi.hoisted(() => ({
  navigate: vi.fn(),
  keys: [] as Array<Record<string, unknown>>,
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mocks.navigate,
}));

vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({ data: mocks.keys }),
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
