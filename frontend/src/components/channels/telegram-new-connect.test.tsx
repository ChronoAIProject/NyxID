import { useState } from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createRootRoute, createRoute, createRouter, createMemoryHistory, Outlet, RouterProvider } from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAppForm } from "@/components/ui/form";
import type { CreateChannelBotFormData } from "@/schemas/channels";
import { parseChannelBotSetupPageSearch, channelBotSetupRewrite } from "@/schemas/channel-bot-setup";
import { ChannelBotSetupPage } from "@/pages/channel-bot-setup";
import { TelegramNewConnect } from "./telegram-new-connect";

const mocks = vi.hoisted(() => ({ request: undefined as undefined | { id: string; status: string; label: string; owner_user_id: string }, requestIds: [] as (string | undefined)[], onConnected: vi.fn() }));
vi.mock("@/stores/auth-store", () => ({ useAuthStore: (select: (value: unknown) => unknown) => select({ user: { id: "owner", email: "owner@example.com" }, isAuthenticated: true }) }));
vi.mock("@/hooks/use-channel-platforms", () => ({ useChannelPlatformViews: () => ({ data: { platforms: [{ platform: "telegram-new", display_name: "Telegram", enabled: true, registration: { fields: [], documentation_url: null } }] } }) }));
vi.mock("@/components/layout/dashboard-layout", () => ({ useBreadcrumbLabel: () => undefined }));
vi.mock("@/components/channels/channel-connection-shell", () => ({ ChannelConnectionShell: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
vi.mock("@/hooks/use-telegram-new", () => ({ useTelegramNewConfiguration: (id?: string) => { mocks.requestIds.push(id); return { data: { request: mocks.request }, isPending: false }; } }));
vi.mock("./telegram-new", () => ({ TelegramNew: ({ requestId, onStarted, onCancelled, onConnected }: { requestId?: string; onStarted: (id?: string) => void; onCancelled: () => void; onConnected: (id: string, username: string) => void }) => <div><span data-testid="request">{requestId ?? "current"}</span><button type="button" onClick={() => onStarted("11111111-1111-4111-8111-111111111111")}>Start</button><button type="button" onClick={onCancelled}>Cancel</button><button type="button" onClick={() => onConnected("bot-id", "ActualBot")}>Complete</button></div> }));

function Flow({ stayInPlace = false, returnTo }: { stayInPlace?: boolean; returnTo?: string }) {
  const form = useAppForm<CreateChannelBotFormData>({ defaultValues: { platform: "telegram-new", label: "Support" } });
  const [done, setDone] = useState(false);
  return done ? <p>Connected</p> : <TelegramNewConnect platform="telegram-new" label="Support" orgId={null} form={form} fullPage={!stayInPlace} stayInPlace={stayInPlace} returnTo={returnTo} renderFields={() => null} onConnected={(...args) => { mocks.onConnected(...args); setDone(true); }}>{null}</TelegramNewConnect>;
}
vi.mock("@/components/channels/channel-bot-setup", () => ({ ChannelBotSetup: ({ returnTo }: { returnTo?: string }) => <Flow returnTo={returnTo} /> }));

function mount(entry: string, inPlace = false) {
  const root = createRootRoute({ component: Outlet });
  const assistant = createRoute({ getParentRoute: () => root, path: "/assistant", component: () => <Flow stayInPlace /> });
  const setup = createRoute({ getParentRoute: () => root, path: "/channel-bots/connect/$platform", validateSearch: parseChannelBotSetupPageSearch, component: ChannelBotSetupPage });
  const router = createRouter({ routeTree: root.addChildren([assistant, setup]), history: createMemoryHistory({ initialEntries: [entry] }), rewrite: channelBotSetupRewrite });
  const client = new QueryClient();
  const view = render(<QueryClientProvider client={client}><RouterProvider router={router} /></QueryClientProvider>);
  return { ...view, router, inPlace };
}
beforeEach(() => { mocks.request = undefined; mocks.requestIds.length = 0; mocks.onConnected.mockClear(); });
afterEach(() => { cleanup(); window.history.replaceState(null, "", "/"); });

it("keeps the chat URL unchanged throughout in-place start, cancel, completion and remount", async () => {
  window.history.replaceState(null, "", "/assistant?c=nyxa-test");
  const location = window.location.href;
  const view = mount("/assistant", true);
  fireEvent.click(await screen.findByText("Start"));
  await waitFor(() => expect(screen.getByTestId("request")).toHaveTextContent("11111111"));
  expect(view.router.state.location.pathname).toBe("/assistant");
  expect(window.location.href).toBe(location);
  fireEvent.click(screen.getByText("Cancel"));
  await waitFor(() => expect(screen.getByTestId("request")).toHaveTextContent("current"));
  view.unmount();
  mocks.request = { id: "11111111-1111-4111-8111-111111111111", status: "waiting_bot", label: "Support", owner_user_id: "owner" };
  mount("/assistant", true);
  await screen.findByText("Start");
  await waitFor(() => expect(mocks.requestIds.at(-1)).toBe("11111111-1111-4111-8111-111111111111"));
  fireEvent.click(screen.getByText("Complete"));
  await screen.findByText("Connected");
  expect(window.location.href).toBe(location);
  expect(mocks.onConnected).toHaveBeenCalledWith("bot-id", true, undefined, "@ActualBot");
});

it.each([`nyxa-${"a".repeat(32)}`, `nyxg-${"b".repeat(32)}`])("preserves %s through router normalization, request insertion, cancellation, completion and refresh", async (target) => {
  const entry = `/channel-bots/connect/telegram-new?return_to=${target}`;
  window.history.replaceState(null, "", entry);
  const view = mount(entry);
  await screen.findByText("Start");
  await waitFor(() => expect(view.router.state.location.search.return_to).toBe(target));
  fireEvent.click(screen.getByText("Start"));
  await waitFor(() => expect(view.router.state.location.search.request_id).toBe("11111111-1111-4111-8111-111111111111"));
  expect(view.router.state.location.search.return_to).toBe(target);
  fireEvent.click(screen.getByText("Cancel"));
  await waitFor(() => expect(view.router.state.location.search.request_id).toBeUndefined());
  expect(view.router.state.location.search.return_to).toBe(target);
  const refresh = view.router.state.location.href;
  view.unmount();
  window.history.replaceState(null, "", refresh);
  const refreshed = mount(refresh);
  fireEvent.click(await screen.findByText("Complete"));
  await screen.findByText("Connected");
  expect(refreshed.router.state.location.search.return_to).toBe(target);
});

it.each(["cancelled", "expired", "suspended"])("keeps Back to chat available in the %s state", async (status) => {
  const target = `nyxa-${"a".repeat(32)}`;
  const entry = `/channel-bots/connect/telegram-new?return_to=${target}`;
  window.history.replaceState(null, "", entry);
  mocks.request = { id: "11111111-1111-4111-8111-111111111111", status, label: "Support", owner_user_id: "owner" };
  mount(entry);
  const back = await screen.findByRole("link", { name: "Back to chat" });
  expect(back).toHaveAttribute("href", expect.stringContaining(target));
});
