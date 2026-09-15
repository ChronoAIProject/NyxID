import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { OAuthClient } from "@/types/api";
import { BrandingCard } from "./branding-card";

const mocks = vi.hoisted(() => ({ upload: vi.fn(), update: vi.fn() }));
vi.mock("@/hooks/use-oauth-branding", () => ({
  useUploadAppLogo: () => ({ mutate: mocks.upload, isPending: false }),
}));
vi.mock("@/hooks/use-developer-apps", () => ({
  useUpdateDeveloperApp: () => ({
    mutateAsync: mocks.update,
    isPending: false,
  }),
}));
const app = {
  id: "app",
  client_name: "App A",
  homepage_url: null,
  logo_url: "/api/v1/branding/assets/logo",
  branding_revision: 3,
  branding_verified_revision: 3,
} as OAuthClient;
beforeEach(() => {
  vi.clearAllMocks();
  mocks.update.mockImplementation(async ({ data }) => ({ ...app, ...data }));
});
describe("White labeling", () => {
  it("previews uploaded identity and clears the chip after a revision change", () => {
    const { rerender } = render(<BrandingCard app={app} />);
    expect(screen.getByText("Verified")).toBeInTheDocument();
    expect(screen.getByRole("img")).toHaveAttribute("src", app.logo_url);
    const file = new File(["image"], "logo.webp", { type: "image/webp" });
    fireEvent.change(screen.getByLabelText("App logo"), {
      target: { files: [file] },
    });
    expect(mocks.upload).toHaveBeenCalledWith(file);
    rerender(<BrandingCard app={{ ...app, branding_revision: 4 }} />);
    expect(screen.queryByText("Verified")).not.toBeInTheDocument();
  });
  it("requires a changed homepage and saves only homepage metadata", async () => {
    const user = userEvent.setup();
    render(<BrandingCard app={app} />);
    expect(
      screen.getByRole("button", { name: "Save homepage" }),
    ).toBeDisabled();
    await user.type(
      screen.getByLabelText("Homepage"),
      "https://app.example.com",
    );
    await user.click(screen.getByRole("button", { name: "Save homepage" }));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith({
        clientId: "app",
        data: { homepage_url: "https://app.example.com" },
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Save homepage" }),
      ).toBeDisabled(),
    );
  });
});
