import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import {
  notificationSchema,
  type NotificationKind,
} from "@/schemas/assistant-notifications";
import { notificationSentence } from "@/lib/assistant/notification-presentation";
import { NotificationRow } from "./notifications-panel";
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
afterEach(cleanup);
const agent = {
  type: "agent",
  id: "researcher",
  name: "researcher",
  display_name: "Researcher",
  kind: "specialist",
};
const fixture = (kind: NotificationKind, extra = {}) =>
  notificationSchema.parse({
    id: "n",
    kind,
    actor: agent,
    source: {
      type: "conversation",
      id: "c",
      label: "Launch plan",
      agent_id: "researcher",
    },
    preview: "The release checklist is ready.",
    count: 3,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    ...extra,
  });
it.each([
  ["automation_result", "Morning digest finished", "automation"],
  ["automation_problem", "Morning digest failed", "automation"],
  ["needs_decision", "Researcher needs your decision in Launch plan", "agent"],
  ["agent_update", "Researcher reported back in Launch plan", "agent"],
  ["chat_update", "Researcher replied in Launch plan on Lark", "agent"],
  ["group_mention", "Researcher mentioned you in Launch plan", "agent"],
] as const)(
  "renders %s with its avatar, badge and sentence",
  (kind, sentence, avatar) => {
    const item = fixture(kind, {
      platform: "lark",
      ...(avatar === "automation"
        ? {
            actor: {
              type: "automation",
              id: "trigger",
              name: "Morning digest",
            },
          }
        : {}),
    });
    const { container } = render(
      <ul>
        <NotificationRow item={item} onOpen={vi.fn()} />
      </ul>,
    );
    const row = screen.getByRole("button", {
      name: `${sentence}, unread${kind === "needs_decision" ? ", needs your decision" : ""}`,
    });
    expect(row).toHaveTextContent(sentence);
    expect(row).toHaveTextContent("The release checklist is ready.");
    expect(row).toHaveTextContent("3 new");
    expect(
      container.querySelector(`[data-notification-avatar="${avatar}"]`),
    ).toBeTruthy();
    expect(
      container.querySelector(`[data-notification-badge="${kind}"]`),
    ).toBeTruthy();
    if (avatar === "agent")
      expect(
        container.querySelector("[data-notification-avatar] > span"),
      ).toHaveClass("h-8", "w-8");
    if (kind === "chat_update")
      expect(container.querySelector('[data-slug="api-lark"]')).toBeTruthy();
  },
);
it("uses the sidebar's crew stack and falls back to the system identity", () => {
  const item = fixture("group_mention", {
    source: {
      type: "collaboration_group",
      id: "g",
      label: "Launch crew",
      group_id: "g",
    },
    participants: [agent, { ...agent, id: "writer", name: "Writer" }],
  });
  const { container, rerender } = render(
    <ul>
      <NotificationRow item={item} onOpen={vi.fn()} />
    </ul>,
  );
  expect(
    container.querySelector('[data-notification-avatar="crew"] > span'),
  ).toHaveClass("-space-x-1.5");
  expect(screen.getByRole("button").querySelectorAll("strong")).toHaveLength(2);
  rerender(
    <ul>
      <NotificationRow
        item={fixture("agent_update", {
          actor: { type: "system", name: "NyxID" },
        })}
        onOpen={vi.fn()}
      />
    </ul>,
  );
  expect(
    container.querySelector('[data-notification-avatar="system"]'),
  ).toBeTruthy();
  expect(screen.getByRole("button")).toHaveTextContent(
    "NyxID replied in Launch plan",
  );
});
it.each([
  [{ outcome: "paused" }, "was paused"],
  [{ outcome: "skipped", reason: "queue_full" }, "was skipped"],
  [
    { outcome: "failed", reason: "result_delivery_failed" },
    "could not deliver its result",
  ],
])(
  "uses stable terminal codes for automation wording (%j)",
  (extra, action) => {
    const item = fixture("automation_problem", {
      actor: { type: "automation", name: "Digest" },
      ...extra,
    });
    expect(notificationSentence(item).text).toBe(`Digest ${action}`);
  },
);

it.each(["group", "collaboration_group"] as const)(
  "reuses the stacked avatar for %s sources, including decisions",
  (type) => {
    const item = fixture("needs_decision", {
      source: { type, id: "g", group_id: "g", label: "Launch crew" },
      participants: [agent],
    });
    const { container } = render(
      <ul>
        <NotificationRow item={item} onOpen={vi.fn()} />
      </ul>,
    );
    expect(
      container.querySelector('[data-notification-avatar="crew"] > span'),
    ).toHaveClass("-space-x-1.5");
    expect(
      container.querySelector('[data-notification-badge="needs_decision"]'),
    ).toBeTruthy();
    expect(screen.getByRole("button")).toHaveTextContent(
      "Researcher needs your decision in Launch crew",
    );
  },
);
it.each(["channel_chat", "conversation"] as const)(
  "uses the agent and platform mark for %s chat sources",
  (type) => {
    const item = fixture("chat_update", {
      source: { type, id: "c", conversation_id: "c", label: "Launch room" },
      platform: "telegram",
    });
    const { container } = render(
      <ul>
        <NotificationRow item={item} onOpen={vi.fn()} />
      </ul>,
    );
    expect(
      container.querySelector('[data-notification-avatar="agent"]'),
    ).toBeTruthy();
    expect(
      container.querySelector('[data-slug="api-telegram-bot"]'),
    ).toBeTruthy();
    expect(screen.getByRole("button")).toHaveTextContent(
      "Researcher replied in Launch room on Telegram",
    );
  },
);
