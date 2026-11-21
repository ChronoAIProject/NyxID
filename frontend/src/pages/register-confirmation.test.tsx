import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { RegisterPage } from "./register";

vi.mock("@/components/auth/auth-flow", () => ({
  AuthFlow: () => <div>Registration</div>,
}));

const initialAuth = useAuthStore.getState();
const profile: User = {
  id: "person",
  display_name: "Amélie",
  email: "person@example.test",
  avatar_url: null,
  social_provider: "google",
  is_active: true,
  is_admin: false,
  email_verified: true,
  mfa_enabled: false,
  created_at: "2026-01-01T00:00:00Z",
};
const transport = vi.fn<typeof fetch>();

function open(query: string, user: User | null = profile, isLoading = false) {
  window.history.replaceState({}, "", `/register?${query}`);
  const navigate = vi
    .spyOn(window.location, "replace")
    .mockImplementation(() => {});
  vi.stubGlobal("fetch", transport);
  transport.mockImplementation(async (_url, options) =>
    Response.json({
      id: profile.id,
      email: profile.email,
      avatar_url: null,
      ...JSON.parse(String(options?.body)),
    }),
  );
  useAuthStore.setState({ user, isAuthenticated: user !== null, isLoading });
  const client = new QueryClient();
  render(
    <QueryClientProvider client={client}>
      <RegisterPage />
    </QueryClientProvider>,
  );
  return navigate;
}

afterEach(() => {
  window.history.replaceState({}, "", "/");
  useAuthStore.setState(initialAuth);
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
  transport.mockReset();
});

it.each([false, true])(
  "saves the exact display name and resumes the original URL (edited %s)",
  async (edit) => {
    const destination = "/oauth/authorize?client_id=test&state=a%2Bb";
    const navigate = open(
      `confirm_name=provider&return_to=${encodeURIComponent(destination)}&display_name=from-link`,
    );
    const input = screen.getByRole("textbox", { name: "Display name" });
    expect(input).toHaveValue("Amélie");
    expect(transport).not.toHaveBeenCalled();
    if (edit) {
      await userEvent.clear(input);
      await userEvent.type(input, "小明");
    }
    await userEvent.click(
      screen.getByRole("button", { name: "Save and continue" }),
    );
    await waitFor(() =>
      expect(navigate).toHaveBeenCalledExactlyOnceWith(
        window.location.origin + destination,
      ),
    );
    expect(transport).toHaveBeenCalledExactlyOnceWith(
      "/api/v1/users/me",
      expect.objectContaining({
        method: "PUT",
        body: JSON.stringify({ display_name: edit ? "小明" : "Amélie" }),
      }),
    );
  },
);

it.each([
  ["apple", null],
  ["github", "octocat"],
])(
  "starts empty without a provider name on %s and allows Skip after an empty Save",
  async (provider, name) => {
    const navigate = open(
      "confirm_name=empty&return_to=https%3A%2F%2Fother.example",
      {
        ...profile,
        social_provider: provider,
        display_name: name,
      },
    );
    expect(screen.getByLabelText("Display name")).toHaveValue("");
    await userEvent.click(
      screen.getByRole("button", { name: "Save and continue" }),
    );
    expect(await screen.findByText("Display name is required")).toBeVisible();
    expect(navigate).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Skip" }));
    expect(navigate).toHaveBeenCalledExactlyOnceWith("/assistant");
    expect(transport).not.toHaveBeenCalled();
  },
);

it("keeps the draft after a failed save and lets the person retry", async () => {
  const navigate = open("confirm_name=empty", {
    ...profile,
    social_provider: "apple",
    display_name: null,
  });
  transport.mockResolvedValueOnce(
    Response.json(
      { error: "internal_error", message: "Failed" },
      { status: 500 },
    ),
  );
  const input = screen.getByLabelText("Display name");
  await userEvent.type(input, "A");
  const save = screen.getByRole("button", { name: "Save and continue" });
  await userEvent.click(save);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Failed to update profile",
  );
  expect(input).toHaveValue("A");
  expect(navigate).not.toHaveBeenCalled();
  await userEvent.click(save);
  await waitFor(() => expect(navigate).toHaveBeenCalledOnce());
  expect(transport).toHaveBeenCalledTimes(2);
});

it("waits for the confirmed session before prefilling", () => {
  open("confirm_name=provider", null, true);
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(screen.queryByText("Registration")).toBeNull();
  act(() =>
    useAuthStore.setState({
      user: profile,
      isAuthenticated: true,
      isLoading: false,
    }),
  );
  expect(screen.getByLabelText("Display name")).toHaveValue(
    profile.display_name,
  );
  expect(transport).not.toHaveBeenCalled();
});

it.each([null, { ...profile, social_provider: null }])(
  "requires a signed-in social user for the hint",
  (user) => {
    open("confirm_name=provider", user);
    expect(screen.getByText("Registration")).toBeVisible();
    expect(transport).not.toHaveBeenCalled();
  },
);

it("requires the callback hint even when a social account is present", () => {
  open("");
  expect(screen.queryByLabelText("Display name")).toBeNull();
  expect(transport).not.toHaveBeenCalled();
});
