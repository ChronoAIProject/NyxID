import { act, cleanup, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { assistantAgentSchema } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { AgentAvatar } from "../nyxbot-agent-avatar";
import {
  recordCreatedAvatar,
  clearCreatedAvatars,
} from "../../../../../shared/agent-avatars/created";
const fixture = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantJson: fixture.request,
}));
vi.mock("@/lib/assistant/use-avatar-environment", () => ({
  useAnimationActive: () => true,
  useReducedAvatarMotion: () => true,
  useDocumentTheme: () => "dark",
}));
afterEach(() => {
  cleanup();
  clearCreatedAvatars();
  useAuthStore.setState({ user: null });
});
it("records a real successful create and presents it once on the focused eligible avatar", async () => {
  useAuthStore.setState({
    user: {
      id: "u",
      capabilities: { enabled_features: ["assistant:avatars"] },
    } as never,
  });
  const agent = assistantAgentSchema.parse({
    id: "created-id",
    name: "new-agent",
    kind: "specialist",
    status: "idle",
    created_at: "2026-11-25T00:00:00Z",
    last_active_at: "2026-11-25T00:00:00Z",
  });
  fixture.request.mockImplementation(
    async (_path: string, options?: { method: string }) =>
      options?.method === "POST"
        ? {
            id: agent.id,
            name: agent.name,
            home_conversation_id: "nyxa-00000000000000000000000000000000",
          }
        : {
            agents: [agent],
            limits: {
              skip_destructive_confirmation: false,
              max_auto_continuations: 4,
              max_live_subagents: 8,
              max_concurrent_subagent_turns: 2,
              max_live_subagents_limit: 20,
              max_concurrent_subagent_turns_limit: 8,
            },
          },
  );
  await nyxBotApi.agents();
  const before = render(<AgentAvatar agent={agent} size="xl" animated />);
  expect(
    before.container.querySelector('[data-avatar-state="created"]'),
  ).toBeNull();
  before.unmount();
  await act(() => nyxBotApi.createAgent({ name: "new-agent" } as never));
  const view = render(
    <>
      <AgentAvatar agent={agent} />
      <AgentAvatar agent={agent} size="xl" animated />
      <AgentAvatar agent={agent} size="xl" animated />
    </>,
  );
  expect(
    view.container.querySelectorAll('[data-avatar-state="created"]'),
  ).toHaveLength(1);
  view.unmount();
  const remount = render(<AgentAvatar agent={agent} size="xl" animated />);
  expect(
    remount.container.querySelector('[data-avatar-state="created"]'),
  ).toBeNull();
});
it("does not create an appearance event for a failed creation", async () => {
  fixture.request.mockRejectedValueOnce(new Error("failed"));
  await expect(
    nyxBotApi.createAgent({ name: "agent" } as never),
  ).rejects.toThrow("failed");
});

it("creation yields permanently to actual approval or stopping", () => {
  useAuthStore.setState({
    user: {
      id: "u",
      capabilities: { enabled_features: ["assistant:avatars"] },
    } as never,
  });
  const agent = assistantAgentSchema.parse({
    id: "created",
    name: "new-agent",
    kind: "specialist",
    status: "idle",
    created_at: "2026-11-25T00:00:00Z",
    last_active_at: "2026-11-25T00:00:00Z",
  });
  recordCreatedAvatar(agent.id);
  const view = render(<AgentAvatar agent={agent} size="xl" animated />);
  expect(
    view.container.querySelector('[data-avatar-state="created"]'),
  ).not.toBeNull();
  view.rerender(
    <AgentAvatar agent={agent} size="xl" animated mood="approval" />,
  );
  expect(
    view.container.querySelector('[data-avatar-state="approval"]'),
  ).not.toBeNull();
  view.rerender(<AgentAvatar agent={agent} size="xl" animated mood="idle" />);
  expect(
    view.container.querySelector('[data-avatar-state="created"]'),
  ).toBeNull();
});

it("does not borrow a creation event when a renderer is reused for another agent", () => {
  useAuthStore.setState({
    user: {
      id: "u",
      capabilities: { enabled_features: ["assistant:avatars"] },
    } as never,
  });
  const agent = (id: string) =>
    assistantAgentSchema.parse({
      id,
      name: id,
      kind: "specialist",
      status: "idle",
      created_at: "2026-11-25T00:00:00Z",
      last_active_at: "2026-11-25T00:00:00Z",
    });
  recordCreatedAvatar("A");
  recordCreatedAvatar("B");
  const ownerB = render(<AgentAvatar agent={agent("B")} size="xl" animated />);
  const reused = render(<AgentAvatar agent={agent("A")} size="xl" animated />);
  expect(
    reused.container.querySelector('[data-avatar-state="created"]'),
  ).not.toBeNull();
  reused.rerender(<AgentAvatar agent={agent("B")} size="xl" animated />);
  expect(
    reused.container.querySelector('[data-avatar-state="created"]'),
  ).toBeNull();
  expect(
    ownerB.container.querySelector('[data-avatar-state="created"]'),
  ).not.toBeNull();
});
