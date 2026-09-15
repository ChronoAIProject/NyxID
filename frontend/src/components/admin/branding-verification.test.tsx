import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AdminOAuthClient } from "@/types/admin";
import { BrandingVerification } from "./branding-verification";
const mutate = vi.fn();
vi.mock("@/hooks/use-oauth-branding", () => ({ useVerifyAppBranding: () => ({ mutate, isPending: false }) }));
describe("admin branding verification", () => {
  it("verifies the displayed revision and supports revoking the mark", async () => {
    const user = userEvent.setup();
    const client = { id: "app", client_name: "App A", branding_revision: 5, branding_verified_revision: 4 } as AdminOAuthClient;
    const { rerender } = render(<BrandingVerification client={client} />);
    const toggle = screen.getByRole("switch", { name: "Verify branding for App A" });
    expect(toggle).not.toBeChecked();
    await user.click(toggle);
    expect(mutate).toHaveBeenLastCalledWith({ branding_revision: 5, verified: true });
    rerender(<BrandingVerification client={{ ...client, branding_verified_revision: 5 }} />);
    expect(toggle).toBeChecked();
    await user.click(toggle);
    expect(mutate).toHaveBeenLastCalledWith({ branding_revision: 5, verified: false });
  });
});
