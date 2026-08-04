import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { grantRevocationDescription } from "@/schemas/oauth-revocation";
import { RevokeConnectionDialog } from "./revoke-connection-dialog";

describe("RevokeConnectionDialog", () => {
  it("presents the grant-revoking consequence as dialog copy", () => {
    render(
      <RevokeConnectionDialog
        providerName="GitHub OAuth"
        revokesGrant
        isPending={false}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("dialog", { name: /Revoke GitHub OAuth connection\?/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(grantRevocationDescription("GitHub OAuth")),
    ).toBeInTheDocument();
  });

  it("says upstream access survives when no grant is revoked", () => {
    render(
      <RevokeConnectionDialog
        providerName="Notion"
        revokesGrant={false}
        isPending={false}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(
      screen.getByText(/Access you granted at Notion stays active/),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(grantRevocationDescription("Notion")),
    ).not.toBeInTheDocument();
  });

  it("names the specific connection when a service has several", () => {
    render(
      <RevokeConnectionDialog
        providerName="GitHub"
        connectionLabel="Work account"
        revokesGrant
        isPending={false}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByText("Work account")).toBeInTheDocument();
  });

  it("routes the two footer actions to their handlers", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(
      <RevokeConnectionDialog
        providerName="GitHub"
        revokesGrant
        isPending={false}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Revoke" }));
    expect(onConfirm).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("blocks cancel while the revoke request is in flight", () => {
    render(
      <RevokeConnectionDialog
        providerName="GitHub"
        revokesGrant
        isPending
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  });
});
