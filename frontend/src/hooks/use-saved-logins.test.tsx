import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { SavedLogin } from "@/schemas/saved-logins";
import { useAuthStore } from "@/stores/auth-store";
import { saveLogin, useSavedLogins } from "./use-saved-logins";
const requests = vi.hoisted(() => ({
  get: vi.fn(),
  post: vi.fn(),
  put: vi.fn(),
}));
vi.mock("@/lib/api-client", () => ({ api: requests }));
const originalAuth = useAuthStore.getState();
afterEach(() => {
  cleanup();
  useAuthStore.setState(originalAuth);
  vi.clearAllMocks();
});
it("uses full PUT for credential replacement and never retains secret variables", async () => {
  useAuthStore.setState({ user: { id: "owner" } as never });
  let sent: Record<string, unknown> | undefined;
  let requestBody: Record<string, unknown> | undefined;
  requests.put.mockImplementationOnce((_url, body) => {
    requestBody = body;
    sent = { ...body };
    return Promise.resolve({ id: "login-id" });
  });
  const values = {
    label: "New label",
    allowed_origins: ["https://example.test"],
    username: "owner",
    password: "secret",
    totp_secret: "",
    confirm_each_sign_in: true,
  };
  await act(async () => {
    await saveLogin(values, "login-id");
  });
  expect(requests.put).toHaveBeenCalledWith(
    "/saved-logins/login-id",
    expect.any(Object),
  );
  expect(sent).toEqual({
    label: "New label",
    allowed_origins: ["https://example.test"],
    username: "owner",
    password: "secret",
    totp_secret: null,
    confirm_each_sign_in: true,
  });
  expect(requestBody).toMatchObject({
    username: "",
    password: null,
    totp_secret: null,
  });
  expect(values).toEqual({
    label: "New label",
    allowed_origins: ["https://example.test"],
    username: "owner",
    password: "secret",
    totp_secret: "",
    confirm_each_sign_in: true,
  });
});

it("includes the actor in the saved-login query key for account isolation", async () => {
  useAuthStore.setState({ user: { id: "first-owner" } as never });
  requests.get.mockResolvedValue([]);
  const client = new QueryClient();
  const { result } = renderHook(() => useSavedLogins(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  await act(async () => {
    await result.current.refetch();
  });
  await waitFor(() => expect(requests.get).toHaveBeenCalled());
  expect(requests.get).toHaveBeenCalledWith("/saved-logins");
  expect(client.getQueryCache().getAll()[0]?.queryKey).toEqual([
    "saved-logins",
    "first-owner",
    null,
  ]);
});

const firstLogin: SavedLogin = {
  id: "first-login",
  owner_id: "first-owner",
  label: "First account login",
  allowed_origins: ["https://first.example.test"],
  username_hint: "••••••",
  has_password: true,
  has_totp: false,
  confirm_each_sign_in: false,
  created_at: "2026-10-29T00:00:00Z",
  updated_at: "2026-10-29T00:00:00Z",
  last_used_at: null,
};
const secondLogin: SavedLogin = {
  ...firstLogin,
  id: "second-login",
  owner_id: "second-owner",
  label: "Second account login",
  allowed_origins: ["https://second.example.test"],
};

it("hides cached personal logins immediately when the authenticated account changes", async () => {
  useAuthStore.setState({ user: { id: "first-owner" } as never });
  let resolveSecond!: (logins: SavedLogin[]) => void;
  requests.get
    .mockResolvedValueOnce([firstLogin])
    .mockImplementationOnce(
      () => new Promise<SavedLogin[]>((resolve) => (resolveSecond = resolve)),
    );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const { result, unmount } = renderHook(() => useSavedLogins(), {
    wrapper: ({ children }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  });
  try {
    await waitFor(() => expect(result.current.data).toEqual([firstLogin]));
    act(() => {
      useAuthStore.setState({ user: { id: "second-owner" } as never });
    });
    await waitFor(() => expect(requests.get).toHaveBeenCalledTimes(2));
    expect(result.current.data).toBeUndefined();
    expect(result.current.isPending).toBe(true);
    await act(async () => resolveSecond([secondLogin]));
    await waitFor(() => expect(result.current.data).toEqual([secondLogin]));
  } finally {
    unmount();
    client.clear();
  }
});

it("excludes a late response from the previous owner after the selected owner changes", async () => {
  useAuthStore.setState({ user: { id: "account-admin" } as never });
  let resolveFirst!: (logins: SavedLogin[]) => void;
  requests.get
    .mockImplementationOnce(
      () => new Promise<SavedLogin[]>((resolve) => (resolveFirst = resolve)),
    )
    .mockResolvedValueOnce([secondLogin]);
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const { result, rerender, unmount } = renderHook(
    ({ owner }) => useSavedLogins(owner),
    {
      initialProps: { owner: "first-owner" },
      wrapper: ({ children }) => (
        <QueryClientProvider client={client}>{children}</QueryClientProvider>
      ),
    },
  );
  try {
    await waitFor(() => expect(requests.get).toHaveBeenCalledTimes(1));
    rerender({ owner: "second-owner" });
    await waitFor(() => expect(result.current.data).toEqual([secondLogin]));
    await act(async () => resolveFirst([firstLogin]));
    await waitFor(() => expect(client.isFetching()).toBe(0));
    expect(result.current.data).toEqual([secondLogin]);
    expect(result.current.data).not.toContainEqual(firstLogin);
  } finally {
    unmount();
    client.clear();
  }
});
