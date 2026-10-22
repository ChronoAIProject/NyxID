import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { LoginIdentityCard } from "./login-identity-card";
import type { useLoginApproval } from "@/hooks/use-login-approval";

vi.mock("@tanstack/react-router", () => ({ Link: () => null }));
vi.mock("./login-methods", () => ({ LoginMethods: () => null }));

describe("social login approval MFA", () => {
  it("offers recovery verification after social identity without requesting a password", async () => {
    const user = userEvent.setup();
    const mfa = vi.fn().mockResolvedValue({ verified: true, mfa_required: false });
    const verified = vi.fn();
    const approval = { identity: { mfa_required: true, verified: false }, restoring: false, mfa } as unknown as ReturnType<typeof useLoginApproval>;
    render(<LoginIdentityCard approval={approval} onVerified={verified} disabled={false} />);
    expect(screen.queryByLabelText("Password")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Use a recovery code" }));
    await user.type(screen.getByLabelText("Recovery code"), "Ab_cd-12");
    await user.click(screen.getByRole("button", { name: "Verify & continue" }));
    await waitFor(() => expect(mfa).toHaveBeenCalledWith("Ab_cd-12"));
    expect(verified).toHaveBeenCalledOnce();
  });
});
