import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { ChannelAdapterResources } from "./channel-adapter-resources";
const mock = vi.hoisted(() => ({
  configure: vi.fn(),
  allocate: vi.fn(),
  revoke: vi.fn(),
  removeAccount: vi.fn(),
}));
const catalog = "11111111-1111-4111-8111-111111111111";
const owner = "22222222-2222-4222-8222-222222222222";
vi.mock("@/hooks/use-services", () => ({
  useServices: () => ({
    data: [
      {
        id: "11111111-1111-4111-8111-111111111111",
        slug: "telnyx",
        name: "Telnyx platform",
        is_active: true,
      },
    ],
  }),
}));
vi.mock("@/hooks/use-admin", () => ({
  useAdminUsers: () => ({
    data: {
      users: [
        {
          id: "22222222-2222-4222-8222-222222222222",
          email: "owner@example.com",
          is_active: true,
        },
      ],
    },
  }),
}));
vi.mock("@/hooks/use-channel-adapters", () => ({
  useAdminAdapterResources: () => ({
    accounts: {
      data: [
        {
          id: "trusted",
          catalog_service_id: "11111111-1111-4111-8111-111111111111",
          provider: "telnyx",
          account_id: "account-one",
          revision: 2,
          webhook_key_configured: true,
        },
      ],
    },
    allocations: {
      data: [
        {
          id: "allocated",
          provider: "telnyx",
          phone_number: "+18005550123",
          owner_id: "22222222-2222-4222-8222-222222222222",
          resource_id: "number-one",
          application_id: "app-one",
          tariff_version: "2026-10",
          tariff_source: "https://telnyx.com/pricing",
          includes_all_provider_charges: true,
        },
      ],
    },
    configure: { mutateAsync: mock.configure, isPending: false },
    allocate: { mutateAsync: mock.allocate, isPending: false },
    revoke: { mutateAsync: mock.revoke, isPending: false },
    removeAccount: { mutateAsync: mock.removeAccount, isPending: false },
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.getState().setUser({
    id: owner,
    email: "admin@example.com",
    display_name: "Admin",
    avatar_url: null,
    email_verified: true,
    mfa_enabled: false,
    is_admin: true,
    is_active: true,
    created_at: "2026-10-17T00:00:00Z",
  });
});
afterEach(cleanup);
async function choose(
  user: ReturnType<typeof userEvent.setup>,
  label: string,
  option: string,
) {
  await user.click(screen.getByRole("combobox", { name: label }));
  await user.click(await screen.findByRole("option", { name: option }));
}
it("requires admin authority to expose trust and allocation management", () => {
  useAuthStore
    .getState()
    .setUser({ ...useAuthStore.getState().user!, is_admin: false });
  render(<ChannelAdapterResources />);
  expect(screen.queryByRole("region")).not.toBeInTheDocument();
  expect(screen.queryByText("Trusted accounts")).not.toBeInTheDocument();
});
it("configures selected platform account trust and its verification key", async () => {
  const user = userEvent.setup();
  render(<ChannelAdapterResources />);
  await user.click(screen.getByText("Configure or rotate account trust"));
  await choose(user, "Platform catalog service", "Telnyx platform");
  await user.type(screen.getByLabelText("Provider account ID"), "new-account");
  await user.type(
    screen.getByLabelText("Account Ed25519 public key"),
    "fixture-public-key",
  );
  await user.click(
    screen.getByRole("button", { name: "Save trusted account" }),
  );
  await waitFor(() =>
    expect(mock.configure).toHaveBeenCalledExactlyOnceWith({
      adapter_id: "telnyx.messaging",
      catalog_service_id: catalog,
      account_id: "new-account",
      webhook_public_key: "fixture-public-key",
    }),
  );
});
it("requires explicit complete-price attestation before allocating to a selected person", async () => {
  const user = userEvent.setup();
  render(<ChannelAdapterResources />);
  await user.click(
    screen.getByText("Allocate a verified number and application"),
  );
  await choose(user, "Trusted account", "account-one");
  await choose(user, "Owner", "owner@example.com");
  for (const [label, value] of [
    ["Phone number (+country code)", "+18005550124"],
    ["Provider number resource ID", "number-two"],
    ["Provider application / messaging profile ID", "app-two"],
    ["Tariff version", "2026-10"],
    ["Official tariff evidence URL", "https://telnyx.com/pricing"],
  ])
    await user.type(screen.getByLabelText(label!), value!);
  const submit = screen.getByRole("button", { name: "Verify and allocate" });
  expect(submit).toBeDisabled();
  await user.click(screen.getByRole("checkbox"));
  await waitFor(() => expect(submit).toBeEnabled());
  await user.click(submit);
  await waitFor(() =>
    expect(mock.allocate).toHaveBeenCalledExactlyOnceWith({
      setup: {
        adapter_id: "telnyx.messaging",
        credential: { kind: "platform", catalog_service_id: catalog },
        account_id: "account-one",
        phone_number: "+18005550124",
        resource_id: "number-two",
        application_id: "app-two",
      },
      owner_id: owner,
      tariff_version: "2026-10",
      tariff_source: "https://telnyx.com/pricing",
      includes_all_provider_charges: true,
    }),
  );
});
it("shows allocation ownership and requires review before revocation", async () => {
  const user = userEvent.setup();
  render(<ChannelAdapterResources />);
  expect(screen.getByText("Owner: owner@example.com")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Revoke allocation" }));
  expect(mock.revoke).not.toHaveBeenCalled();
  const dialog = screen.getByRole("dialog");
  expect(dialog).toHaveTextContent(
    "all sessions and admitted usage are finalized",
  );
  await user.click(
    within(dialog).getByRole("button", { name: "Confirm removal" }),
  );
  await waitFor(() =>
    expect(mock.revoke).toHaveBeenCalledExactlyOnceWith("allocated"),
  );
});
