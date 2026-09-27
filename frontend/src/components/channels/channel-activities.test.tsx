import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import type {
  ChannelActivityDescriptor,
  ChannelActivityItem,
  ChannelActivityResponse,
} from "@/types/channels";
import {
  ActivityCallbackSettings,
  ChannelActivities,
  LatestActivity,
} from "./channel-activities";

const descriptors: readonly ChannelActivityDescriptor[] = [
  {
    kind: "encrypted_chat",
    label: "Encrypted chat",
    subscription: "chat",
    description: "Encrypted notifications",
    content_availability: "encrypted",
    reply_supported: false,
  },
];
const item: ChannelActivityItem = {
  id: "activity",
  conversation_id: "route",
  platform_conversation_id: "chat:10:20",
  platform_event_id: "event",
  sender_platform_id: "20",
  kind: "encrypted_chat",
  provider_event_type: "chat.received",
  content_availability: "encrypted",
  reply_supported: false,
  callback_status: "not_enabled",
  received_at: new Date().toISOString(),
  occurred_at: null,
};
const response: ChannelActivityResponse = {
  activities: [item],
  total: 1,
  retention_days: 30,
  routes: [{ conversation_id: "route", count: 1, last_activity: item }],
  page: 1,
  per_page: 20,
};
const clients: QueryClient[] = [];
function mount(element: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  clients.push(client);
  return render(
    <QueryClientProvider client={client}>{element}</QueryClientProvider>,
  );
}
afterEach(() => {
  cleanup();
  clients.splice(0).forEach((client) => client.clear());
  vi.restoreAllMocks();
});

describe("channel activity observations", () => {
  it("counts encrypted activity before an agent callback is enabled", async () => {
    vi.spyOn(api, "get").mockResolvedValue(response);
    mount(<ChannelActivities scope="bot" id="bot" descriptors={descriptors} />);
    expect(
      await screen.findByText("1 received activity in the last 30 days"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Agent notification not enabled"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "Encrypted message received. Text and replies are unavailable.",
      ),
    ).toBeInTheDocument();
  });
  it("shows an error rather than a false zero or an empty observation", async () => {
    vi.spyOn(api, "get").mockRejectedValue(new Error("unavailable"));
    mount(<ChannelActivities scope="bot" id="bot" descriptors={descriptors} />);
    expect(
      await screen.findByText("Activity count unavailable"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/0 received activities/)).not.toBeInTheDocument();
    expect(screen.queryByText(/No matching activity/)).not.toBeInTheDocument();
  });
  it("labels a receipt as accepted without asserting agent processing", async () => {
    vi.spyOn(api, "get").mockResolvedValue({
      ...response,
      activities: [{ ...item, callback_status: "delivered" }],
    });
    mount(
      <ChannelActivities scope="route" id="route" descriptors={descriptors} />,
    );
    expect(await screen.findByText("Callback accepted")).toBeInTheDocument();
    expect(screen.queryByText("Processing")).not.toBeInTheDocument();
  });
  it("uses the platform's activity labels for the latest received resource", () => {
    const view = render(
      <LatestActivity activity={item} descriptors={descriptors} />,
    );
    expect(screen.getByText(/Encrypted chat ·/)).toBeInTheDocument();
    view.rerender(
      <LatestActivity activity={item} descriptors={descriptors} unavailable />,
    );
    expect(screen.getByText("Activity unavailable")).toBeInTheDocument();
  });
});

describe("activity callback consent", () => {
  it("does not allow enabling an undeclared receiver", async () => {
    vi.spyOn(api, "get").mockResolvedValue({
      declared: false,
      enabled: false,
      kinds: [],
      version: null,
    });
    mount(<ActivityCallbackSettings id="route" descriptors={descriptors} />);
    expect(
      await screen.findByRole("button", { name: "Enable typed callbacks" }),
    ).toBeDisabled();
  });
  it("enables only after a receiver declaration and explicit owner action", async () => {
    const get = vi
      .spyOn(api, "get")
      .mockResolvedValue({
        declared: true,
        enabled: false,
        kinds: ["encrypted_chat"],
        version: 1,
      });
    const put = vi.spyOn(api, "put").mockImplementation(async () => {
      get.mockResolvedValue({
        declared: true,
        enabled: true,
        kinds: ["encrypted_chat"],
        version: 1,
      });
    });
    mount(<ActivityCallbackSettings id="route" descriptors={descriptors} />);
    const button = await screen.findByRole("button", {
      name: "Enable typed callbacks",
    });
    expect(put).not.toHaveBeenCalled();
    await userEvent.click(button);
    await waitFor(() =>
      expect(put).toHaveBeenCalledWith(
        "/channel-conversations/route/activity-callback",
        { enabled: true },
      ),
    );
    expect(
      await screen.findByRole("button", { name: "Disable typed callbacks" }),
    ).toBeEnabled();
  });
});
