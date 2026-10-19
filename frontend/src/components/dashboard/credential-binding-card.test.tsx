import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { KeyInfo } from "@/types/keys";
import { CredentialBindingCard } from "./credential-binding-card";
const { mutateAsync, mutate } = vi.hoisted(() => ({
  mutateAsync: vi.fn(),
  mutate: vi.fn(),
}));
vi.mock("@/hooks/use-keys", () => ({
  useUpdateKey: () => ({ mutateAsync, mutate, isPending: false }),
}));
vi.mock("./add-key-dialog", () => ({ AddKeyDialog: () => null }));
const service = {
  id: "service",
  credential_binding: "user",
  platform_key_available: true,
  is_active: true,
  auto_connected: false,
} as KeyInfo;
beforeEach(() => {
  vi.clearAllMocks();
  mutateAsync.mockResolvedValue({ status: "active" });
});
it("switches to platform without sending the retained personal key", async () => {
  render(<CredentialBindingCard service={service} readOnly={false} />);
  await userEvent.click(
    screen.getByRole("button", { name: "Use NyxID's key" }),
  );
  expect(mutateAsync).toHaveBeenCalledWith({
    keyId: "service",
    use_platform_key: true,
  });
});
it("requires a replacement credential to switch to BYOK and supports disable", async () => {
  render(
    <CredentialBindingCard
      service={{ ...service, credential_binding: "platform" }}
      readOnly={false}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Use your own key" }),
  ).toBeDisabled();
  await userEvent.type(
    screen.getByLabelText("Your replacement credential"),
    "new-personal-secret",
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Use your own key" }),
  );
  expect(mutateAsync).toHaveBeenCalledWith({
    keyId: "service",
    use_platform_key: false,
    credential: "new-personal-secret",
  });
  await userEvent.click(screen.getByRole("button", { name: "Disable" }));
  expect(mutate).toHaveBeenCalledWith(
    { keyId: "service", is_active: false },
    expect.any(Object),
  );
});
it("prevents changes for org members without write access", () => {
  render(<CredentialBindingCard service={service} readOnly />);
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});
it("collects custom OAuth app inputs when switching from platform", async () => {
  render(
    <CredentialBindingCard
      service={{ ...service, credential_binding: "platform" }}
      catalog={
        {
          provider_type: "oauth2",
          credential_mode: "user",
        } as import("@/types/keys").CatalogEntry
      }
      readOnly={false}
    />,
  );
  expect(
    screen.getByRole("button", { name: "Use your own key" }),
  ).toBeDisabled();
  await userEvent.type(
    screen.getByLabelText("Your OAuth app client ID"),
    "custom-app",
  );
  await userEvent.type(
    screen.getByLabelText("Your OAuth app client secret"),
    "custom-secret",
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Use your own key" }),
  );
  expect(mutateAsync).toHaveBeenCalledWith({
    keyId: "service",
    use_platform_key: false,
    oauth_client_id: "custom-app",
    oauth_client_secret: "custom-secret",
  });
});
