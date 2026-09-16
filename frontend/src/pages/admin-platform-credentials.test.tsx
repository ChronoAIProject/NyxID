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
  isLoading: false,
  error: null as unknown,
}));
vi.mock("@/hooks/use-admin-platform-credentials", () => ({
  useAdminPlatformCredentials: () => ({
    data: mock.data,
    isLoading: mock.isLoading,
    error: mock.error,
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
