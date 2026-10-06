import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { SavedLoginsPage } from "./saved-logins";
const mock = vi.hoisted(() => ({ save: vi.fn(), remove: vi.fn() }));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: () => <span>Personal account</span>,
}));
vi.mock("@/hooks/use-saved-logins", () => ({
  useSavedLogins: () => ({
    data: [
      {
        id: "login",
        label: "Website",
        allowed_origins: ["https://example.test"],
        username_hint: "••••",
        has_password: true,
        has_totp: true,
        confirm_each_sign_in: false,
      },
    ],
  }),
  saveLogin: mock.save,
  useDeleteSavedLogin: () => ({ mutateAsync: mock.remove }),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
it("shows metadata only, replaces write-only secrets and clears the form after saving", async () => {
  const client = new QueryClient();
  const user = userEvent.setup();
  mock.save.mockResolvedValue({});
  render(
    <QueryClientProvider client={client}>
      <SavedLoginsPage />
    </QueryClientProvider>,
  );
  expect(screen.getByText(/deliberately/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Replace" }));
  expect(screen.getByLabelText("Username")).toHaveValue("");
  expect(screen.getByLabelText("Username")).toHaveAttribute("type", "text");
  expect(screen.getByLabelText("Username")).toHaveAttribute(
    "autocomplete",
    "off",
  );
  expect(screen.getByLabelText("Password (optional)")).toHaveAttribute(
    "type",
    "password",
  );
  await user.type(screen.getByLabelText("Username"), "owner@example.test");
  await user.type(
    screen.getByLabelText("Password (optional)"),
    "synthetic-test-password",
  );
  await user.click(screen.getByRole("button", { name: "Save login" }));
  await waitFor(() => expect(mock.save).toHaveBeenCalledOnce());
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(JSON.stringify(client.getQueryCache().getAll())).not.toContain(
    "synthetic-test-password",
  );
});
it("requires a delete confirmation before removing a login and its grants", async () => {
  mock.remove.mockResolvedValue({});
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SavedLoginsPage />
    </QueryClientProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Delete" }));
  expect(mock.remove).not.toHaveBeenCalled();
  expect(
    screen.getByText(/removes the saved login and its specialist grants/),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Delete login" }));
  await waitFor(() => expect(mock.remove).toHaveBeenCalledWith("login"));
});

it("shows a validation message for each invalid origin line", async () => {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <SavedLoginsPage />
    </QueryClientProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Replace" }));
  fireEvent.change(screen.getByLabelText("Username"), {
    target: { value: "owner" },
  });
  const origins = screen.getByLabelText("Allowed HTTPS origins (one per line)");
  expect(origins.tagName).toBe("TEXTAREA");
  fireEvent.change(origins, {
    target: {
      value: "https://valid.test\nhttp://insecure.test\nhttps://bad.test/path",
    },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save login" }));
  await waitFor(() => expect(screen.getByText(/^Line 2:/)).toBeInTheDocument());
  expect(screen.getByText(/^Line 3:/)).toBeInTheDocument();
  expect(mock.save).not.toHaveBeenCalled();
});
