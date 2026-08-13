import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { grantRevocationDescription } from "@/schemas/oauth-revocation";
import { DeleteConnectionDialog } from "./delete-connection-dialog";

describe("DeleteConnectionDialog", () => {
  it("presents the grant-revoking consequence as dialog copy", () => {
    render(
      <DeleteConnectionDialog
        providerName="GitHub OAuth"
        revokesGrant
        isPending={false}
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("dialog", { name: /Delete GitHub OAuth connection\?/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(grantRevocationDescription("GitHub OAuth")),
    ).toBeInTheDocument();
  });

  it("says upstream access survives when no grant is revoked", () => {
    render(
      <DeleteConnectionDialog
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
      <DeleteConnectionDialog
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
      <DeleteConnectionDialog
        providerName="GitHub"
        revokesGrant
        isPending={false}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />,
    );

    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(onConfirm).toHaveBeenCalledOnce();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("blocks cancel while the delete request is in flight", () => {
    render(
      <DeleteConnectionDialog
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
