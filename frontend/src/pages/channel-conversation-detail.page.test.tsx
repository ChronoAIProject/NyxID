import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  RouterProvider,
} from "@tanstack/react-router";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import { ChannelConversationDetailPage } from "./channel-conversation-detail";

vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn() } }));

const conversation = {
  id: "conv-1",
  channel_bot_id: "bot-1",
  platform: "telegram",
  platform_conversation_id: "123",
  platform_conversation_type: "private",
  platform_sender_id: null,
  agent_api_key_id: "agent",
  default_agent: false,
  is_active: true,
  allow_agent_initiated: true,
  capabilities: {
    media: { inbound: [], outbound: [] },
    initiated_send: true,
    reply_to: true,
    thread: true,
    edit: false,
  },
  last_message_at: null,
  created_at: "2026-09-15T00:00:00Z",
  updated_at: "2026-09-15T00:00:00Z",
};
const bot = {
  id: "bot-1",
  platform: "telegram",
  label: "Support",
  platform_bot_id: "1",
  platform_bot_username: "support_bot",
  webhook_registered: true,
  status: "active",
  is_active: true,
  created_at: "2026-09-15T00:00:00Z",
  updated_at: "2026-09-15T00:00:00Z",
  user_id: "org-1",
};
const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });

let botAvailable = false;
const sends: unknown[] = [];

beforeEach(() => {
  transitionAssistantIdentity("person-a");
  useCreditsDenialStore.getState().reset();
  botAvailable = false;
  sends.length = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init?: RequestInit) => {
      const path = String(url).replace("/api/v1", "");
      if (path === "/channel-bots/bot-1") {
        return botAvailable
          ? json(bot)
          : json(
              { error: "internal_error", error_code: 1000, message: "x" },
              500,
            );
      }
      if (path.startsWith("/channel-conversations/")) return json(conversation);
      if (path === "/channel-platforms") return json({ platforms: [] });
      if (path === "/channel-relay/send") {
        sends.push(JSON.parse(String(init?.body)));
        return json(
          {
            error: "insufficient_credits",
            error_code: 11300,
            message: "Insufficient credits",
          },
          402,
        );
      }
      return json({ messages: [], total: 0 });
    }),
  );
});
afterEach(() => {
  vi.unstubAllGlobals();
  transitionAssistantIdentity(null);
});

async function renderPage() {
  const root = createRootRoute();
  const route = createRoute({
    getParentRoute: () => root,
    path: "/channel-bots/$botId/conversations/$conversationId",
    component: ChannelConversationDetailPage,
  });
  const router = createRouter({
    routeTree: root.addChildren([route]),
    history: createMemoryHistory({
      initialEntries: ["/channel-bots/bot-1/conversations/conv-1"],
    }),
  });
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
  await act(() => router.load());
}

it("recovers from a failed bot load with Retry, then bills sends to the bot's org", async () => {
  await renderPage();
  // The bot query failed: an error with Retry, not a loading hint.
  expect(
    await screen.findByText(
      "This bot's details could not be loaded, so test messages can't be sent.",
    ),
  ).toBeInTheDocument();
  expect(
    screen.queryByText("Sending is available once this bot's details load."),
  ).toBeNull();
  expect(
    screen.getByRole("button", { name: "Send test message" }),
  ).toBeDisabled();

  botAvailable = true;
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() =>
    expect(
      screen.queryByText(
        "This bot's details could not be loaded, so test messages can't be sent.",
      ),
    ).toBeNull(),
  );
  fireEvent.change(screen.getByLabelText("Send test message"), {
    target: { value: "hello" },
  });
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Send test message" }),
    ).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "Send test message" }));
  await waitFor(() => expect(sends).toHaveLength(1));
  await waitFor(() =>
    expect(useCreditsDenialStore.getState().current).toMatchObject({
      payer: { org: { id: "org-1" } },
      key: expect.stringMatching(/^op:channel-relay-send:conv-1:/),
    }),
  );
});
