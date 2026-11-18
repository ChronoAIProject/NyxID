import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { AgentVoiceService } from "./agent-voice-service";

const mocks = vi.hoisted(() => ({ enabled: true, request: vi.fn() }));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => mocks.enabled,
}));
vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantJson: mocks.request,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, ...props }: ComponentProps<"a"> & { to: string }) => (
    <a href={to} {...props} />
  ),
}));
const id = "99df3b18-c1df-4fa7-af45-04c724d66c77";
const service = {
  id: "dd8c62cc-c9b2-4fea-858a-d349c098c143",
  slug: "nyxbot-voice-example",
  agent_id: id,
};
const agent = {
  id,
  name: "personal-helper",
  kind: "specialist",
  owner_kind: "person",
  status: "idle",
} as AssistantAgent;

function owner(id: string) {
  useAuthStore.setState({
    user: { ...useAuthStore.getState().user, id } as NonNullable<
      ReturnType<typeof useAuthStore.getState>["user"]
    >,
  });
}
function show(value = agent) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <AgentVoiceService agent={value} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  owner("owner-a");
  mocks.enabled = true;
  mocks.request.mockReset();
});
afterEach(cleanup);

it("creates no connection or Key grant on load; publication is explicit and names one Bot", async () => {
  mocks.request
    .mockResolvedValueOnce({ service: null })
    .mockResolvedValueOnce({ service })
    .mockResolvedValue({ service });
  show();
  const button = await screen.findByRole("button", {
    name: "Create voice connection",
  });
  await waitFor(() => expect(button).toBeEnabled());
  expect(mocks.request).toHaveBeenCalledTimes(1);
  expect(
    screen.getByText(/Other Bots require their own connections/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/Keys set to all services include it/),
  ).toBeInTheDocument();
  await userEvent.click(button);
  await screen.findByRole("link", { name: "Manage Agent Key access" });
  const writes = mocks.request.mock.calls.filter(
    ([, options]) => options?.method,
  );
  expect(writes).toEqual([
    [
      `/assistant/nyxagent/agents/${id}/voice-service`,
      { method: "PUT", authorityGuard: expect.any(Function) },
    ],
  ]);
});

it("does not expose publication for shared, organization or destroyed Bots", () => {
  for (const fields of [
    { share_role: "manager" },
    { owner_kind: "org" },
    { status: "destroyed" },
  ]) {
    show({ ...agent, ...fields } as AssistantAgent);
    expect(
      screen.queryByRole("region", { name: "Device voice" }),
    ).not.toBeInTheDocument();
    cleanup();
  }
  expect(mocks.request).not.toHaveBeenCalled();
});

it("keeps publication disabled while the feature is off", () => {
  mocks.enabled = false;
  show();
  expect(mocks.request).not.toHaveBeenCalled();
});

it("discloses the primary NyxBot's full account and service authority", async () => {
  mocks.request.mockResolvedValue({ service });
  show({ ...agent, kind: "nyxbot" });
  await screen.findByRole("link", { name: "Manage Agent Key access" });
  expect(
    screen.getByText(/This NyxBot has full account and service access/),
  ).toBeInTheDocument();
});
