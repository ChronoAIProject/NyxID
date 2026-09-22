import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { PlatformCredentials } from "@/types/admin";
import { AdminPlatformCredentialsPage } from "./admin-platform-credentials";
const mock = vi.hoisted(() => ({
  data: [] as PlatformCredentials[],
  update: vi.fn(),
  clear: vi.fn(),
  reset: vi.fn(),
  refetch: vi.fn(),
  isLoading: false,
  error: null as unknown,
}));
vi.mock("@/hooks/use-admin-platform-credentials", () => ({
  useAdminPlatformCredentials: () => ({
    data: mock.data,
    isLoading: mock.isLoading,
    error: mock.error,
    refetch: mock.refetch,
  }),
  useUpdatePlatformCredentials: () => ({
    mutateAsync: mock.update,
    isPending: false,
    reset: mock.reset,
  }),
  useClearPlatformCredentials: () => ({
    mutateAsync: mock.clear,
    isPending: false,
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.isLoading = false;
  mock.error = null;
  mock.data = [
    {
      provider: "future-provider",
      platform: "future",
      label: "Future Provider",
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
      updated_at: "v1",
    },
  ];
  mock.update.mockImplementation(async ({ fields }) => {
    const saved = {
      ...mock.data[0]!,
      updated_at: "v2",
      fields: mock.data[0]!.fields.map((f) =>
        fields && f.name in fields
          ? {
              ...f,
              value: f.secret ? undefined : (fields[f.name] ?? undefined),
              configured: fields[f.name] !== null,
            }
          : f,
      ),
    };
    mock.data = [saved];
    return saved;
  });
});
it("renders saved non-secret values and configured secret status", () => {
  render(<AdminPlatformCredentialsPage />);
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
it("reviews only changed credentials and resets the baseline after success", async () => {
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Tenant ID"), {
    target: { value: "tenant-43" },
  });
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const dialog = await screen.findByRole("dialog", { name: "Review changes" });
  expect(within(dialog).getByText("tenant-42")).toBeInTheDocument();
  expect(within(dialog).getByText("tenant-43")).toBeInTheDocument();
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(
    within(dialog).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledExactlyOnceWith({
      fields: { tenant: "tenant-43" },
    }),
  );
  expect(
    screen.getByRole("button", { name: "Save credentials" }),
  ).toBeDisabled();
});
it("confirms secret removal as an explicit null and never renders a replacement secret", async () => {
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Signing Key"), {
    target: { value: "new-secret-never-preview" },
  });
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const dialog = await screen.findByRole("dialog");
  expect(dialog).not.toHaveTextContent("new-secret-never-preview");
  expect(within(dialog).getByText("Replace stored value")).toBeInTheDocument();
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await user.click(screen.getByRole("button", { name: "Clear Signing Key" }));
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  expect(await screen.findByText("Clear stored value")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledExactlyOnceWith({
      fields: { signing_key: null },
    }),
  );
});
it("keeps edits during refresh and blocks saving until the latest values are loaded", async () => {
  const user = userEvent.setup();
  const view = render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Tenant ID"), {
    target: { value: "local-draft" },
  });
  mock.data = [
    {
      ...mock.data[0]!,
      updated_at: "v3",
      fields: mock.data[0]!.fields.map((f) =>
        f.name === "tenant" ? { ...f, value: "remote-change" } : f,
      ),
    },
  ];
  view.rerender(<AdminPlatformCredentialsPage />);
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("local-draft");
  expect(
    screen.getByRole("button", { name: "Save credentials" }),
  ).toBeDisabled();
  await user.click(
    screen.getByRole("button", { name: "Load latest values (discard edits)" }),
  );
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("remote-change");
  expect(mock.update).not.toHaveBeenCalled();
});

it("retains a credential draft when a background refresh fails", () => {
  const view = render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Tenant ID"), {
    target: { value: "unsaved-tenant" },
  });
  mock.error = new Error("Network unavailable");
  view.rerender(<AdminPlatformCredentialsPage />);
  expect(
    screen.getByText("Unable to refresh platform credentials"),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("unsaved-tenant");
});

it("preserves unsaved credential edits when regenerating the verify token", async () => {
  mock.data[0] = { ...mock.data[0]!, webhook_verify_token: "old-verify-token" };
  const user = userEvent.setup();
  const view = render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Tenant ID"), {
    target: { value: "unsaved-tenant" },
  });
  await user.click(
    screen.getByRole("button", { name: "Regenerate verify token" }),
  );
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  view.rerender(<AdminPlatformCredentialsPage />);
  expect(mock.update).toHaveBeenCalledExactlyOnceWith({
    regenerate_verify_token: true,
  });
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("unsaved-tenant");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save credentials" }),
    ).toBeEnabled(),
  );
});

it("blocks duplicate confirmation and retains the reviewed draft after a failed save", async () => {
  const user = userEvent.setup();
  let rejectSave!: (error: Error) => void;
  mock.update.mockImplementationOnce(
    () =>
      new Promise((_, reject) => {
        rejectSave = reject;
      }),
  );
  render(<AdminPlatformCredentialsPage />);
  fireEvent.change(screen.getByLabelText("Tenant ID"), {
    target: { value: "retry-tenant" },
  });
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  await user.dblClick(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  expect(mock.update).toHaveBeenCalledTimes(1);
  await act(async () => rejectSave(new Error("Save failed")));
  const dialog = screen.getByRole("dialog", { name: "Review changes" });
  expect(within(dialog).getByRole("alert")).toHaveTextContent("Save failed");
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("retry-tenant");
  await user.click(
    within(dialog).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(mock.update).toHaveBeenNthCalledWith(2, {
    fields: { tenant: "retry-tenant" },
  });
});

it("sends an explicit null for a cleared non-secret field", async () => {
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  await user.clear(screen.getByLabelText("Tenant ID"));
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  await user.click(
    await screen.findByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledExactlyOnceWith({
      fields: { tenant: null },
    }),
  );
});

it("enables review after one secret paste and warns for shared OAuth field clears", async () => {
  mock.data[0] = {
    ...mock.data[0]!,
    backing: { type: "provider_oauth", provider_slug: "twitter" },
  };
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  await user.click(screen.getByLabelText("Signing Key"));
  await user.paste("one-paste-secret");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Save credentials" }),
    ).toBeEnabled(),
  );
  await user.click(screen.getByRole("button", { name: "Clear Signing Key" }));
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  expect(review).toHaveTextContent("OAuth connections and logins");
  expect(review).not.toHaveTextContent("one-paste-secret");
  await user.click(
    within(review).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledWith({ fields: { signing_key: null } }),
  );
});

it("offers only a read refresh after a successful clear with failed refresh", async () => {
  mock.clear.mockResolvedValue({ saved: null });
  mock.refetch.mockResolvedValue({
    data: [{ ...mock.data[0]!, updated_at: "restored" }],
    error: null,
  });
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  await user.click(screen.getByRole("button", { name: "Clear provider" }));
  await user.click(screen.getByRole("button", { name: "Confirm" }));
  await screen.findByText(
    "Credentials cleared; current details could not be refreshed",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Clear provider" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: /Retry/ }));
  await waitFor(() => expect(mock.refetch).toHaveBeenCalledTimes(1));
  expect(mock.clear).toHaveBeenCalledTimes(1);
});

it("renders Aurinko's three application fields and reviews a signing-only clear accurately", async () => {
  mock.data = [
    {
      provider: "aurinko",
      platform: "aurinko",
      label: "Aurinko Email",
      available: true,
      backing: { type: "provider_oauth", provider_slug: "aurinko" },
      fields: [
        ["client_id", "Application Client ID"],
        ["client_secret", "Application Client Secret"],
        ["signing_secret", "Application webhook signing secret"],
      ].map(([name, label]) => ({
        name: name!,
        label: label!,
        secret: true,
        required: true,
        numeric: false,
        configured: true,
        help: "From the Aurinko application",
      })),
      setup_checklist: ["Each mailbox requires its owner's authorization."],
      callback_url: null,
      webhook_verify_token: null,
      updated_at: "v1",
    },
  ];
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  expect(
    screen.getByRole("heading", { name: "Aurinko Email" }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Clear provider" }));
  const clear = await screen.findByRole("dialog", {
    name: "Clear platform credentials",
  });
  expect(clear).toHaveTextContent("prevents managed mailbox authorization");
  expect(clear).toHaveTextContent(
    "Manual connections keep their own credentials",
  );
  expect(clear).not.toHaveTextContent(
    "all of its OAuth connections and logins",
  );
  await user.click(within(clear).getByRole("button", { name: "Cancel" }));
  for (const label of [
    "Application Client ID",
    "Application Client Secret",
    "Application webhook signing secret",
  ]) {
    expect(screen.getByLabelText(label)).toHaveAttribute("type", "password");
    expect(screen.getByLabelText(label)).toHaveValue("");
  }
  await user.click(
    screen.getByRole("button", {
      name: "Clear Application webhook signing secret",
    }),
  );
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  expect(review).toHaveTextContent("AI Service mailbox tokens are retained");
  expect(review).not.toHaveTextContent("OAuth connections and logins");
  await user.click(
    within(review).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledWith({
      fields: { signing_secret: null },
    }),
  );
});

function xProvider(): PlatformCredentials {
  return {
    provider: "x",
    platform: "x",
    label: "X (Twitter)",
    available: true,
    backing: { type: "provider_oauth", provider_slug: "twitter" },
    fields: [
      ["app_bearer_token", "App bearer token"],
      ["consumer_secret", "API key secret"],
      ["client_id", "Client ID"],
      ["client_secret", "Client Secret"],
    ].map(([name, label]) => ({
      name: name!,
      label: label!,
      secret: true,
      required: name === "client_id" || name === "client_secret",
      numeric: false,
      configured: true,
      help: "From the X app",
    })),
    setup_checklist: ["Fund the shared app."],
    callback_url: null,
    webhook_verify_token: null,
    updated_at: "v1",
  };
}

it.each([
  ["API key secret", "consumer_secret", "stops verified DM webhook delivery"],
  [
    "App bearer token",
    "app_bearer_token",
    "stops subscription setup and cleanup",
  ],
])("explains the impact of clearing the X %s", async (label, field, impact) => {
  mock.data = [xProvider()];
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  await user.click(screen.getByRole("button", { name: `Clear ${label}` }));
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  expect(review).toHaveTextContent("X webhook credentials");
  expect(review).toHaveTextContent(impact);
  if (field === "app_bearer_token") {
    expect(review).not.toHaveTextContent("stops verified DM webhook delivery");
    expect(review).toHaveTextContent("continue delivering billable events");
  }
  expect(review).toHaveTextContent("do not fall back to polling");
  expect(review).toHaveTextContent("OAuth connections and logins are retained");
  expect(review).not.toHaveTextContent("stops all of the twitter provider's");
  await user.click(
    within(review).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledWith({
      fields: { [field]: null },
    }),
  );
});

it("keeps the shared OAuth impact for X OAuth field clears and the whole provider", async () => {
  mock.data = [xProvider()];
  const user = userEvent.setup();
  render(<AdminPlatformCredentialsPage />);
  await user.click(screen.getByRole("button", { name: "Clear provider" }));
  const clear = await screen.findByRole("dialog", {
    name: "Clear platform credentials",
  });
  expect(clear).toHaveTextContent(
    "shared with the twitter provider. Clearing them stops all of its OAuth connections and logins",
  );
  await user.click(within(clear).getByRole("button", { name: "Cancel" }));
  await user.click(screen.getByRole("button", { name: "Clear Client Secret" }));
  await user.click(
    screen.getByRole("button", { name: "Clear App bearer token" }),
  );
  await user.click(screen.getByRole("button", { name: "Save credentials" }));
  const review = await screen.findByRole("dialog", { name: "Review changes" });
  expect(review).toHaveTextContent("Shared OAuth credentials");
  expect(review).toHaveTextContent(
    "stops all of the twitter provider's OAuth connections and logins",
  );
  expect(review).toHaveTextContent("stops subscription setup and cleanup");
  expect(review).not.toHaveTextContent(
    "OAuth connections and logins are retained",
  );
  await user.click(
    within(review).getByRole("button", { name: "Confirm changes" }),
  );
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledWith({
      fields: { client_secret: null, app_bearer_token: null },
    }),
  );
});
