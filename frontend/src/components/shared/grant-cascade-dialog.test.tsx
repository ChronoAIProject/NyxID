import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { GRANT_CASCADE_CAVEAT } from "@/schemas/oauth-revocation";
import { GrantCascadeDialog } from "./grant-cascade-dialog";

const details = {
  provider_slug: "github",
  provider_name: "GitHub",
  revokes_grant: true,
  siblings: [
    {
      user_service_id: "service-2",
      name: "GitHub Issues",
      slug: "github-issues",
    },
  ],
  unaffected_other_app: [
    {
      user_service_id: "service-3",
      name: "Enterprise GitHub",
      slug: "github-enterprise",
    },
  ],
  token_scope_available: true,
};

describe("GrantCascadeDialog", () => {
  it("renders server-provided services, fixed caveat, and both recovery actions", async () => {
    const user = userEvent.setup();
    const onCascade = vi.fn();
    const onRemoveOnly = vi.fn();
    render(
      <GrantCascadeDialog
        details={details}
        isPending={false}
        onCascade={onCascade}
        onRemoveOnly={onRemoveOnly}
        onCancel={vi.fn()}
      />,
    );

    expect(screen.getByText("GitHub Issues")).toBeInTheDocument();
    expect(
      screen.getByText("not affected (different OAuth app): Enterprise GitHub"),
    ).toBeInTheDocument();
    expect(screen.getByText(GRANT_CASCADE_CAVEAT)).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Remove only this service" }),
    );
    await user.click(
      screen.getByRole("button", {
        name: "Disconnect GitHub everywhere (2 services)",
      }),
    );
    expect(onRemoveOnly).toHaveBeenCalledOnce();
    expect(onCascade).toHaveBeenCalledOnce();
  });

  it("replaces token-scope copy with an honest local-only action", () => {
    render(
      <GrantCascadeDialog
        details={{ ...details, token_scope_available: false }}
        isPending={false}
        onCascade={vi.fn()}
        onRemoveOnly={vi.fn()}
        onCancel={vi.fn()}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Remove from NyxID only" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Remove only this service"),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText(/authorization will remain active at GitHub/i),
    ).toBeInTheDocument();
  });
});
