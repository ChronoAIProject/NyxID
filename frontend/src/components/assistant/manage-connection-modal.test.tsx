import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import type { KeyInfo } from "@/types/keys";

const mocks = vi.hoisted(() => ({
  useKey: vi.fn(),
  useCatalogEntry: vi.fn(),
  useUpdateKey: vi.fn(),
  useDeleteKey: vi.fn(),
  useUpdateExternalApiKey: vi.fn(),
}));

vi.mock("@/hooks/use-keys", () => ({
  useKey: mocks.useKey,
  useCatalogEntry: mocks.useCatalogEntry,
  useUpdateKey: mocks.useUpdateKey,
  useDeleteKey: mocks.useDeleteKey,
  useUpdateExternalApiKey: mocks.useUpdateExternalApiKey,
}));

// The reconnect wizard has its own suite and a deep hook tree; stub it to a
// marker so these tests can assert only that Reconnect hands the right key to
// the same dialog the Studio page uses.
vi.mock("@/components/dashboard/add-key-dialog", () => ({
  AddKeyDialog: ({
    open,
    reconnectKey,
  }: {
    readonly open: boolean;
    readonly reconnectKey?: { readonly id: string } | null;
  }) =>
    open ? (
      <div data-testid="reconnect-dialog" data-key={reconnectKey?.id ?? ""} />
    ) : null,
}));

vi.mock("@/components/service-icon", () => ({
  ServiceIcon: ({ slug }: { readonly slug: string }) => <span>{slug}</span>,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
  }: {
    readonly to: string;
    readonly params?: Record<string, string>;
    readonly children?: ReactNode;
  }) => (
    <a href="#" data-to={to} data-params={JSON.stringify(params ?? null)}>
      {children}
    </a>
  ),
}));

import { ManageConnectionModal } from "./manage-connection-modal";

/** A GitHub OAuth connection whose authorization never landed — the exact
 *  shape the backend leaves behind when a callback comes back denied. */
const failedOAuthKey = {
  id: "key-1",
  label: "GitHub OAuth",
  slug: "github",
  credential_type: "oauth2",
  catalog_service_slug: "api-github",
  catalog_service_name: "GitHub OAuth",
  status: "failed",
  is_active: true,
  last_used_at: null,
  error_message: "access_denied: The user denied the request",
  granted_scopes: null,
} as unknown as KeyInfo;

function renderModal(overrides: Partial<KeyInfo> = {}, keyIds = ["key-1"]) {
  mocks.useKey.mockReturnValue({
    data: { ...failedOAuthKey, ...overrides },
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  });
  return render(
    <ManageConnectionModal
      keyIds={keyIds}
      serviceName="GitHub OAuth"
      iconSlug="api-github"
      onClose={vi.fn()}
    />,
  );
}

describe("ManageConnectionModal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.useCatalogEntry.mockReturnValue({
      data: {
        slug: "api-github",
        name: "GitHub OAuth",
        provider_type: "oauth2",
      },
    });
    mocks.useUpdateKey.mockReturnValue({ mutate: vi.fn(), isPending: false });
    mocks.useDeleteKey.mockReturnValue({ mutate: vi.fn(), isPending: false });
    mocks.useUpdateExternalApiKey.mockReturnValue({
      mutate: vi.fn(),
      isPending: false,
    });
  });

  describe("broken-connection explanation", () => {
    it("says what 'failed' means instead of showing a bare status pill", () => {
      renderModal();
      expect(
        screen.getByText(/Authorization never completed/i),
      ).toBeInTheDocument();
    });

    it("shows the provider's own reason recorded on the connection", () => {
      renderModal();
      expect(
        screen.getByText("access_denied: The user denied the request"),
      ).toBeInTheDocument();
    });

    it("offers reconnect as the way out of a failed OAuth connection", async () => {
      const user = userEvent.setup();
      renderModal();
      await user.click(screen.getByRole("button", { name: /Reconnect/ }));
      expect(screen.getByTestId("reconnect-dialog")).toHaveAttribute(
        "data-key",
        "key-1",
      );
    });

    it("calls a half-finished authorization what it is", () => {
      renderModal({ status: "pending_auth", error_message: null });
      expect(
        screen.getByRole("button", { name: /Continue authentication/ }),
      ).toBeInTheDocument();
      expect(
        screen.getByText(/hasn't sent NyxID a credential/i),
      ).toBeInTheDocument();
    });

    it("explains a revoked connection without offering a dead-end reconnect", () => {
      // Re-authorizing cannot resurrect a revoked credential — the user needs
      // a new connection, so a Reconnect button here would only fail.
      renderModal({ status: "revoked", error_message: null });
      expect(screen.getByText(/can no longer be used/i)).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /Reconnect/ }),
      ).not.toBeInTheDocument();
    });

    it("stays quiet for a healthy connection", () => {
      renderModal({ status: "active", error_message: null });
      expect(screen.queryByText(/Authorization never completed/i)).toBeNull();
      expect(screen.queryByRole("button", { name: /Reconnect/ })).toBeNull();
    });

    it("does not offer reconnect on a pasted API key", () => {
      // An api_key credential is repaired by replacing the secret, not by
      // walking an OAuth flow.
      mocks.useCatalogEntry.mockReturnValue({
        data: { slug: "openai", name: "OpenAI", provider_type: null },
      });
      renderModal({
        credential_type: "api_key",
        api_key_id: "api-key-1",
        catalog_service_slug: "openai",
      });
      expect(screen.queryByRole("button", { name: /Reconnect/ })).toBeNull();
      expect(
        screen.getByRole("button", { name: "Replace" }),
      ).toBeInTheDocument();
    });

    it("still explains the failure when the connection is read-only", () => {
      // Org members can't fix it themselves, but they should know why the
      // assistant can't use the service.
      renderModal({ auto_connected: true });
      expect(
        screen.getByText(/Authorization never completed/i),
      ).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /Reconnect/ })).toBeNull();
    });
  });

  describe("delete confirmation", () => {
    it("confirms in a dialog rather than swapping the card footer", async () => {
      const user = userEvent.setup();
      renderModal();
      await user.click(screen.getByRole("button", { name: "Delete" }));

      const confirm = await screen.findByRole("dialog", {
        name: /Delete GitHub OAuth connection\?/,
      });
      expect(
        within(confirm).getByRole("button", { name: "Cancel" }),
      ).toBeInTheDocument();
    });

    it("keeps the upstream de-authorization warning inside that dialog", async () => {
      const user = userEvent.setup();
      renderModal({ revocation: { revokes_grant: true } } as Partial<KeyInfo>);
      // Before confirming, the sentence must not be loose in the card — that
      // is what made it read as an error rather than a prompt.
      expect(screen.queryByText(/de-authorizes NyxID/)).toBeNull();

      await user.click(screen.getByRole("button", { name: "Delete" }));
      const confirm = await screen.findByRole("dialog", {
        name: /Delete GitHub OAuth connection\?/,
      });
      expect(
        within(confirm).getByText(/de-authorizes NyxID/),
      ).toBeInTheDocument();
    });

    it("deletes only after the dialog is confirmed", async () => {
      const mutate = vi.fn();
      mocks.useDeleteKey.mockReturnValue({ mutate, isPending: false });
      const user = userEvent.setup();
      renderModal();

      await user.click(screen.getByRole("button", { name: "Delete" }));
      expect(mutate).not.toHaveBeenCalled();

      const confirm = await screen.findByRole("dialog", {
        name: /Delete GitHub OAuth connection\?/,
      });
      await user.click(within(confirm).getByRole("button", { name: "Delete" }));
      expect(mutate).toHaveBeenCalledWith("key-1", expect.anything());
    });

    it("dismisses without deleting when cancelled", async () => {
      const mutate = vi.fn();
      mocks.useDeleteKey.mockReturnValue({ mutate, isPending: false });
      const user = userEvent.setup();
      renderModal();

      await user.click(screen.getByRole("button", { name: "Delete" }));
      const confirm = await screen.findByRole("dialog", {
        name: /Delete GitHub OAuth connection\?/,
      });
      await user.click(within(confirm).getByRole("button", { name: "Cancel" }));

      expect(mutate).not.toHaveBeenCalled();
      expect(
        screen.queryByRole("dialog", {
          name: /Delete GitHub OAuth connection\?/,
        }),
      ).toBeNull();
    });

    it("names the connection when a service has more than one", async () => {
      const user = userEvent.setup();
      mocks.useKey.mockImplementation((keyId: string) => ({
        data: {
          ...failedOAuthKey,
          id: keyId,
          label: keyId === "key-1" ? "Work account" : "Personal account",
        },
        isLoading: false,
        error: null,
        refetch: vi.fn(),
      }));
      render(
        <ManageConnectionModal
          keyIds={["key-1", "key-2"]}
          serviceName="GitHub OAuth"
          iconSlug="api-github"
          onClose={vi.fn()}
        />,
      );

      await user.click(
        screen.getAllByRole("button", { name: "Delete" })[0] as HTMLElement,
      );
      const confirm = await screen.findByRole("dialog", {
        name: /Delete GitHub OAuth connection\?/,
      });
      expect(within(confirm).getByText("Work account")).toBeInTheDocument();
    });
  });
});
