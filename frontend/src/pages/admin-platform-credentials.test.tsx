import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { PlatformCredentials } from "@/types/admin";
import { AdminPlatformCredentialsPage } from "./admin-platform-credentials";

const mocks = vi.hoisted(() => ({
  providers: [] as PlatformCredentials[],
  update: vi.fn(),
  reset: vi.fn(),
}));

vi.mock("@/hooks/use-admin-platform-credentials", () => ({
  useAdminPlatformCredentials: () => ({
    data: mocks.providers,
    isLoading: false,
  }),
  useUpdatePlatformCredentials: () => ({
    mutateAsync: mocks.update,
    reset: mocks.reset,
    isPending: false,
  }),
  useClearPlatformCredentials: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.providers = [
    {
      provider: "future-provider",
      label: "Future Provider",
      platform: "future-platform",
      available: true,
      fields: [
        {
          name: "tenant",
          label: "Tenant ID",
          secret: false,
          configured: true,
          value: "tenant-42",
          help: "Your tenant",
          numeric: false,
          required: true,
        },
        {
          name: "signing_key",
          label: "Signing Key",
          secret: true,
          configured: true,
          help: "Rotate here",
          numeric: false,
          required: true,
        },
      ],
      setup_checklist: ["A provider-owned setup step"],
      callback_url: null,
      webhook_verify_token: null,
      updated_at: "today",
    },
  ];
});

it("renders an unknown provider entirely from its descriptor with masked configured secrets", () => {
  render(<AdminPlatformCredentialsPage />);
  expect(screen.getByText("Future Provider")).toBeInTheDocument();
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("tenant-42");
  expect(screen.getByLabelText("Signing Key")).toHaveAttribute(
    "type",
    "password",
  );
  expect(screen.getByLabelText("Signing Key")).toHaveValue("");
  expect(screen.getByText("A provider-owned setup step")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Save credentials" }),
  ).toBeDisabled();
});

it("saves a Telegram manager token after a single paste", async () => {
  mocks.providers = [
    {
      provider: "telegram-new",
      label: "Telegram — bot creation",
      platform: "telegram-new",
      available: false,
      fields: [
        {
          name: "manager_bot_token",
          label: "Manager bot token",
          secret: true,
          configured: false,
          help: "Dedicated manager bot token",
          numeric: false,
          required: true,
        },
      ],
      setup_checklist: [],
      callback_url: null,
      webhook_verify_token: null,
      updated_at: null,
    },
  ];
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  const save = screen.getByRole("button", { name: "Save credentials" });
  expect(save).toBeDisabled();

  await user.click(screen.getByLabelText("Manager bot token"));
  await user.paste("123456789:synthetic-manager-token");

  await waitFor(() => expect(save).toBeEnabled());
  await user.click(save);
  await waitFor(() =>
    expect(mocks.update).toHaveBeenCalledExactlyOnceWith({
      fields: { manager_bot_token: "123456789:synthetic-manager-token" },
    }),
  );
});

it("saves only the rotated secret and disables saving invalid or unchanged values", async () => {
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  const secret = screen.getByLabelText("Signing Key");
  const save = screen.getByRole("button", { name: "Save credentials" });

  await user.click(secret);
  await user.paste("replacement-secret");
  await waitFor(() => expect(save).toBeEnabled());
  await user.click(save);
  await waitFor(() =>
    expect(mocks.update).toHaveBeenCalledExactlyOnceWith({
      fields: { signing_key: "replacement-secret" },
    }),
  );

  fireEvent.change(secret, { target: { value: "x".repeat(4097) } });
  await waitFor(() => expect(save).toBeDisabled());
  await user.clear(secret);
  await waitFor(() => expect(save).toBeDisabled());
  await user.paste("valid-replacement");
  await waitFor(() => expect(save).toBeEnabled());
});
