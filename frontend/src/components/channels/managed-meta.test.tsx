import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { ChannelConnectLinkContext, useChannelConnectLink } from "@/hooks/use-channel-connect-link";
import { ManagedMetaConnect } from "./managed-meta";

vi.mock("@/stores/auth-store", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/stores/auth-store")>()),
  useAuthStore: (select: (state: { user: { id: string } }) => unknown) =>
    select({ user: { id: "owner" } }),
}));

afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.restoreAllMocks();
});

it("keeps each inbox's saved consent session and resets asset confirmation on navigation", async () => {
  const client = new QueryClient();
  const first = "587da031-d973-493f-985a-9765d3c8bde0";
  const second = "2d2c5f73-d1e1-43a8-869b-b8493b472967";
  const key = (id: string) => `meta-inbox:owner:messenger:personal:${id}:direct`;
  sessionStorage.setItem(key("a"), first);
  sessionStorage.setItem(key("b"), second);
  vi.spyOn(api, "get").mockImplementation(async (path) => {
    const id = path.split("/").at(-1);
    return {
      platform: "messenger",
      status: "assets",
      session_id: id,
      bot_id: null,
      assets: [{ id: "123", name: id === first ? "First Page" : "Second Page" }],
    };
  });
  const form = (botId: string) => (
    <QueryClientProvider client={client}>
      <ManagedMetaConnect platform="messenger" label="Support" orgId={null} botId={botId} onConnected={vi.fn()} />
    </QueryClientProvider>
  );
  const { rerender, unmount } = render(form("a"));
  await userEvent.click(await screen.findByRole("radio", { name: /First Page/ }));
  await userEvent.click(screen.getByRole("checkbox"));
  rerender(form("b"));
  expect(await screen.findByRole("radio", { name: /Second Page/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Connect selected inbox" })).toBeDisabled();
  expect(sessionStorage.getItem(key("b"))).toBe(second);
  unmount();
  client.clear();
});

it("follows a replacement session on the same tracked link and clears the old confirmation", async () => {
  const first = "587da031-d973-493f-985a-9765d3c8bde0";
  const next = "2d2c5f73-d1e1-43a8-869b-b8493b472967";
  const client = new QueryClient();
  vi.spyOn(api, "get").mockImplementation(async (path) => ({
    session_id: path.split("/").at(-1),
    platform: "messenger",
    status: "assets",
    bot_id: null,
    assets: [{ id: "123", name: path.endsWith(first) ? "Initial Page" : "Replacement Page" }],
  }));
  const form = (connectionId: string) => (
    <QueryClientProvider client={client}>
      <ChannelConnectLinkContext.Provider value={{ id: "link", token: "fixture", connectionId, refresh: vi.fn() }}>
        <ManagedMetaConnect platform="messenger" label="Support" orgId={null} onConnected={vi.fn()} />
      </ChannelConnectLinkContext.Provider>
    </QueryClientProvider>
  );
  const { rerender, unmount } = render(form(first));
  await userEvent.click(await screen.findByRole("radio", { name: /Initial Page/ }));
  await userEvent.click(screen.getByRole("checkbox"));
  rerender(form(next));
  expect(await screen.findByRole("radio", { name: /Replacement Page/ })).not.toBeChecked();
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Connect selected inbox" })).toBeDisabled();
  unmount();
  client.clear();
});

it.each(["assets", "installing"] as const)("prevents a finish from the old %s session while restarting sign-in", async (status) => {
  const id = "587da031-d973-493f-985a-9765d3c8bde0";
  sessionStorage.setItem("meta-inbox:owner:messenger:personal:new:direct", id);
  const client = new QueryClient();
  vi.spyOn(api, "get").mockResolvedValue({
    session_id: id, platform: "messenger", status, bot_id: null,
    selected_asset_id: "123", assets: [{ id: "123", name: "Page" }],
  });
  const popup = { close: vi.fn(), location: { href: "" } };
  vi.spyOn(window, "open").mockReturnValue(popup as unknown as Window);
  let rejectStart!: (reason: Error) => void;
  vi.spyOn(api, "post").mockImplementation(() => new Promise((_, reject) => { rejectStart = reject; }));
  const { unmount } = render(
    <QueryClientProvider client={client}>
      <ManagedMetaConnect platform="messenger" label="Support" orgId={null} onConnected={vi.fn()} />
    </QueryClientProvider>,
  );
  if (status === "assets") {
    await userEvent.click(await screen.findByRole("radio"));
    await userEvent.click(screen.getByRole("checkbox"));
  }
  const finish = await screen.findByRole("button", { name: status === "assets" ? "Connect selected inbox" : "Retry verification" });
  expect(finish).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "Sign in and grant inbox access" }));
  expect(finish).toBeDisabled();
  await act(async () => rejectStart(new Error("Sign-in unavailable")));
  if (status === "assets") expect(finish).toBeDisabled();
  unmount();
  client.clear();
});

it.each([true, false])("uses the server pin after a concurrent tracked start (local start won: %s)", async (localWon) => {
  const first = "587da031-d973-493f-985a-9765d3c8bde0";
  const other = "2d2c5f73-d1e1-43a8-869b-b8493b472967";
  const local = "16a17d85-5828-4886-9827-0e9eb96070e0";
  const url = "https://example.com/consent";
  const client = new QueryClient();
  const row = {
    id: "link", platform: "messenger", status: "pending", label: "Support",
    owner_id: "owner", requested_by: null, expires_at: "2030-01-01",
    bot_id: null, connection_id: first, telegram_request_id: null,
    callback_url: null, last_error: null, delivery_status: null,
  };
  let pin = first;
  vi.spyOn(api, "get").mockImplementation(async (path) =>
    path === "/channel-connect-links/link" ? { ...row, connection_id: pin } : {
      session_id: path.split("/").at(-1), platform: "messenger",
      status: "pending", bot_id: null, assets: [],
    });
  let finishStart!: () => void;
  vi.spyOn(api, "post").mockImplementation(async (path) => {
    if (path === "/channel-connect-links/preview") return row;
    await new Promise<void>((resolve) => { finishStart = resolve; });
    return { session_id: local, authorization_url: url };
  });
  const popup = { close: vi.fn(), location: { href: "about:blank" } };
  vi.spyOn(window, "open").mockReturnValue(popup as unknown as Window);
  function Tracked() {
    const link = useChannelConnectLink("fixture", "owner");
    if (!link.status.data) return null;
    return <ChannelConnectLinkContext.Provider value={{
      id: row.id, token: "fixture", connectionId: link.status.data.connection_id ?? undefined, refresh: link.refresh,
    }}>
      <ManagedMetaConnect platform="messenger" label="Support" orgId={null} onConnected={vi.fn()} />
    </ChannelConnectLinkContext.Provider>;
  }
  const { unmount } = render(<QueryClientProvider client={client}><Tracked /></QueryClientProvider>);
  const start = await screen.findByRole("button", { name: "Sign in and grant inbox access" });
  await userEvent.click(start);
  pin = other;
  await act(async () => { await client.refetchQueries({ queryKey: ["channel-connect-link"] }); });
  const storageKey = "meta-inbox:owner:messenger:personal:new:link";
  await waitFor(() => expect(sessionStorage.getItem(storageKey)).toBe(other));
  expect(start).toBeDisabled();
  pin = localWon ? local : other;
  await act(async () => finishStart());
  await waitFor(() => expect(start).toBeEnabled());
  expect(popup.location.href).toBe(localWon ? url : "about:blank");
  expect(popup.close).toHaveBeenCalledTimes(localWon ? 0 : 1);
  expect(sessionStorage.getItem(storageKey)).toBe(pin);
  unmount();
  client.clear();
});
