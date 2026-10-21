import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRootRoute, createRoute, createRouter, createMemoryHistory, Outlet, RouterProvider } from "@tanstack/react-router";
import { afterEach, expect, it, vi } from "vitest";
import { ChannelBotSetup } from "./channel-bot-setup";
import { channelPlatformViews } from "@/lib/channel-platforms";
import type { ChannelPlatformDescriptor } from "@/types/channels";

const mocks = vi.hoisted(() => ({ get: vi.fn(), actor: { id: "owner" } }));
vi.mock("@/lib/api-client", async (original) => ({ ...await original<object>(), api: { get: mocks.get } }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (value: unknown) => unknown) => select({ user: mocks.actor }) }));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: [] }) }));
vi.mock("@/hooks/use-channel-bots", () => ({ useCreateChannelBot: () => ({ mutate: vi.fn(), reset: vi.fn(), isPending: false }), useTelegramBotProfile: () => ({ mutate: vi.fn(), isPending: false }) }));
vi.mock("@/hooks/use-channel-platforms", () => ({ useChannelPlatformViews: () => {
  const descriptor = { platform: "telegram-new", display_name: "Telegram", enabled: true, registration: { fields: [], setup_instructions: [], documentation_url: null } } as unknown as ChannelPlatformDescriptor;
  const platforms = channelPlatformViews([descriptor]);
  return { data: { platforms: [descriptor] }, platforms, getPlatform: (id: string) => platforms[id] };
} }));

afterEach(() => { cleanup(); vi.clearAllMocks(); window.history.replaceState(null, "", "/"); });

it("retains a discovered in-place request through terminal completion using the real Telegram hooks", async () => {
  const id = "11111111-1111-4111-8111-111111111111";
  const request = { id, status: "provisioning", revision: 1, label: "Support", owner_user_id: "owner", expires_at: "2026-10-15T00:00:00Z", telegram_bot_id: "123", bot_username: "ActualBot", channel_bot_id: null, auto_connect: true };
  const config = (value: unknown) => ({ available: true, manager_username: "ManagerBot", request: value });
  let connected = false;
  mocks.get.mockImplementation(async (endpoint: string) => config(endpoint.includes("request_id=") ? (connected ? { ...request, status: "connected", channel_bot_id: id } : request) : (connected ? null : request)));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const close = vi.fn();
  const root = createRootRoute({ component: Outlet });
  const route = createRoute({ getParentRoute: () => root, path: "/assistant", component: () => <ChannelBotSetup open onOpenChange={close} defaultOrgId={null} defaultPlatform="telegram-new" defaultLabel="Support" stayInPlace /> });
  const router = createRouter({ routeTree: root.addChildren([route]), history: createMemoryHistory({ initialEntries: ["/assistant?c=original"] }) });
  window.history.replaceState(null, "", "/assistant?c=original");
  const location = window.location.href;
  const href = router.state.location.href;
  render(<QueryClientProvider client={client}><RouterProvider router={router} /></QueryClientProvider>);
  await waitFor(() => expect(mocks.get).toHaveBeenCalledWith(`/channel-bots/telegram-new?request_id=${id}`));
  connected = true;
  expect(await mocks.get("/channel-bots/telegram-new")).toEqual(config(null));
  await act(async () => { await client.invalidateQueries({ queryKey: ["telegram-new", "owner"] }); });
  expect(await screen.findByText("@ActualBot is connected. NyxBot will continue in this chat.")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Back to chat" }));
  expect(close).toHaveBeenCalledWith(false);
  expect(router.state.location.href).toBe(href);
  expect(window.location.href).toBe(location);
  client.clear();
});
