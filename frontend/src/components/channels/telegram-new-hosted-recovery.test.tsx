import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createMemoryHistory, createRootRoute, createRoute, createRouter, Outlet, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ChannelConnectLinkContent } from "./channel-connect-link-content";
import { channelPlatformViews } from "@/lib/channel-platforms";
import type { ChannelPlatformDescriptor } from "@/types/channels";
import type { TelegramNewRequest } from "@/schemas/telegram-new";

const mocks = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn(), replace: vi.fn(), close: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({ ...await original<object>(), api: { get: mocks.get, post: mocks.post } }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select?: (value: unknown) => unknown) => {
  const state = { user: { id: "owner", email: "owner@example.com" }, isAuthenticated: true, isLoading: false };
  return select ? select(state) : state;
} }));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("@/hooks/use-channel-bots", () => ({ useCreateChannelBot: () => ({ mutate: vi.fn(), reset: vi.fn(), isPending: false }), useTelegramBotProfile: () => ({ mutate: vi.fn(), isPending: false }) }));
vi.mock("@/hooks/use-channel-platforms", () => ({ useChannelPlatformViews: () => {
  const descriptor = { platform: "telegram-new", display_name: "Telegram", enabled: true, registration: { fields: [], setup_instructions: [], documentation_url: null } } as unknown as ChannelPlatformDescriptor;
  const platforms = channelPlatformViews([descriptor]);
  return { data: { platforms: [descriptor] }, platforms, getPlatform: (id: string) => platforms[id] };
} }));
vi.mock("./channel-connection-shell", () => ({ ChannelConnectionShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));

const request: TelegramNewRequest = {
  id: "6d13196a-a0fe-40e8-915a-ff3a77502e42", status: "waiting_bot", revision: 2,
  label: "Merchant support", owner_user_id: "owner", expires_at: "2099-10-27T02:00:00Z",
  telegram_bot_id: null, bot_username: null, channel_bot_id: null, auto_connect: true,
  can_open_creation_form: true,
};
const link = {
  id: "hosted-link", status: "pending", platform: "telegram-new", label: request.label,
  owner_id: "owner", requested_by: "Existing specialist", expires_at: request.expires_at,
  bot_id: null, connection_id: null, telegram_request_id: request.id,
  callback_url: null, last_error: null, delivery_status: null,
};
const creationUrl = "https://t.me/newbot/NyxSetupBot/merchant_bot?name=Merchant%20support";
const config = (saved: TelegramNewRequest | null) => ({ available: true, manager_username: "NyxSetupBot", request: saved });
const progressPath = `/channel-bots/telegram-new?request_id=${request.id}`;

function mount() {
  const root = createRootRoute({ component: Outlet });
  const route = createRoute({ getParentRoute: () => root, path: "/connect/bot/$token", component: () => <ChannelConnectLinkContent token="nyx_bcl_saved" /> });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: ["/connect/bot/nyx_bcl_saved"] }) });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(<QueryClientProvider client={client}><RouterProvider router={router} /></QueryClientProvider>);
  return { ...view, client, router };
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.get.mockImplementation(async (path: string) => path === "/channel-connect-links/hosted-link" ? link : config(request));
  mocks.post.mockImplementation(async (path: string) => {
    if (path === "/channel-connect-links/preview") return link;
    if (path.endsWith("/creation-link")) return { request, creation_url: creationUrl };
    throw new Error(`Unexpected mutation: ${path}`);
  });
  vi.spyOn(window, "open").mockReturnValue({ opener: null, closed: false, location: { replace: mocks.replace }, close: mocks.close } as unknown as Window);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it("checks submitted-form progress from the actual hosted full-page caller without starting or renewing setup", async () => {
  const view = mount();
  const progress = await screen.findByRole("button", { name: "Check progress" });
  const submitted = screen.getByText(/Already submitted Telegram’s form/);
  const alternate = screen.getByText(/same Telegram account on an up-to-date official Telegram client/);
  expect(submitted.compareDocumentPosition(alternate) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(alternate).toHaveTextContent(/existing private chat.*bot keyboard.*\/start once.*same live setup/);
  expect(screen.getByText(/Do not restart NyxBot/)).toBeVisible();
  const reads = mocks.get.mock.calls.filter(([path]) => path === progressPath).length;
  fireEvent.click(progress);
  await waitFor(() => expect(mocks.get.mock.calls.filter(([path]) => path === progressPath).length).toBeGreaterThan(reads));
  expect(view.client.getQueryData(["telegram-new", "owner", request.id, link.id])).toEqual(config(request));
  expect(mocks.post.mock.calls).toEqual([["/channel-connect-links/preview", { token: "nyx_bcl_saved" }]]);
  expect(window.open).not.toHaveBeenCalled();
  expect(view.router.state.location.href).toBe("/connect/bot/nyx_bcl_saved");
  view.client.clear();
});

it("opens the existing hosted request's form and suppresses it when the saved setup closes", async () => {
  vi.mocked(window.open).mockReturnValue(null);
  const view = mount();
  fireEvent.click(await screen.findByRole("button", { name: "Open creation form" }));
  expect(await screen.findByRole("link", { name: "Open Telegram creation form" })).toHaveAttribute("href", creationUrl);
  expect(mocks.post.mock.calls).toEqual([
    ["/channel-connect-links/preview", { token: "nyx_bcl_saved" }],
    [`/channel-bots/telegram-new/requests/${request.id}/creation-link`],
  ]);
  mocks.get.mockImplementation(async (path: string) => path === "/channel-connect-links/hosted-link" ? link : config({
    ...request, setup_notice: {
      reason: "context_changed", message: "Review the saved destination.",
      status_url: "https://app.nyxid.test/saved", new_setup_url: "https://app.nyxid.test/setup", agent_url: null,
    },
  }));
  await act(() => view.client.invalidateQueries({ queryKey: ["telegram-new"] }));
  expect(await screen.findByText("Review the saved destination.")).toHaveAttribute("role", "status");
  expect(screen.queryByRole("link", { name: "Open Telegram creation form" })).not.toBeInTheDocument();
  expect(screen.queryByText(/same Telegram account on an up-to-date/)).not.toBeInTheDocument();
  expect(view.router.state.location.href).toBe("/connect/bot/nyx_bcl_saved");
  view.client.clear();
});

it("retains the normal first-start handoff when the hosted caller adopts its newly saved request", async () => {
  let started = false;
  mocks.get.mockImplementation(async (path: string) => path === "/channel-connect-links/hosted-link"
    ? { ...link, telegram_request_id: started ? request.id : null }
    : config(started ? { ...request, status: "waiting_telegram" } : null));
  mocks.post.mockImplementation(async (path: string) => {
    if (path === "/channel-connect-links/preview") return { ...link, telegram_request_id: null };
    if (path === "/channel-connect-links/telegram/start") {
      started = true;
      return { request: { ...request, status: "waiting_telegram" }, launch_url: "https://t.me/NyxSetupBot?start=saved" };
    }
    throw new Error(`Unexpected mutation: ${path}`);
  });
  const view = mount();
  fireEvent.click(await screen.findByRole("button", { name: "Continue in Telegram" }));
  await waitFor(() => expect(mocks.replace).toHaveBeenCalledExactlyOnceWith("https://t.me/NyxSetupBot?start=saved"));
  expect(mocks.close).not.toHaveBeenCalled();
  expect(mocks.post.mock.calls).toEqual([
    ["/channel-connect-links/preview", { token: "nyx_bcl_saved" }],
    ["/channel-connect-links/telegram/start", { token: "nyx_bcl_saved" }],
  ]);
  await waitFor(() => expect(mocks.get).toHaveBeenCalledWith(progressPath));
  expect(view.router.state.location.href).toBe("/connect/bot/nyx_bcl_saved");
  view.client.clear();
});
