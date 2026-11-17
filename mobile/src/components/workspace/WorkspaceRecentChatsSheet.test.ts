import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { translationTestMocks } from "../../i18n/translationTestHarness";
import { lightColors } from "../../theme/mobileTheme";
import type { WorkspaceRecent } from "./workspaceSidebarModel";

type E = { type: unknown; props: Record<string, any> };
const nodes = (node: any): E[] =>
  node?.props
    ? [node, ...[node.props.children].flat(Infinity).flatMap(nodes)]
    : [];
const text = (node: E): string =>
  [node.props.children].flat(Infinity).map((c: any) =>
    typeof c === "string" ? c : c?.props ? text(c) : "",
  ).join("");

const row = (n: number, target: WorkspaceRecent["target"]): WorkspaceRecent => ({
  key: `row-${n}`,
  title: `Thread ${n}`,
  activityAt: "2026-11-16T00:00:00Z",
  target,
});

function setup(data: Partial<Record<string, unknown>>, markers = {}) {
  const opened: unknown[] = [];
  const recents = {
    rows: [] as WorkspaceRecent[],
    loading: false,
    online: true,
    error: false,
    active: true,
    ...data,
  };
  const h = hookHarness<E>({
    ...translationTestMocks,
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Text: "Text",
      StyleSheet: { create: (v: unknown) => v },
    },
    "@react-navigation/native": { useIsFocused: () => true },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: lightColors }) },
    "../../theme/designTokens": { fonts: {} },
    "../../app/WorkspaceHeader": {
      useWorkspaceSheet: () => ({
        onOpenTarget: (target: unknown) => opened.push(target),
      }),
    },
    "../../features/notifications/useNotifications": {
      useNotificationIndicator: () => ({
        markers: { conversations: markers, groups: {} },
      }),
    },
    "../Skeleton": { SkeletonRows: "SkeletonRows" },
    "./WorkspaceUI": { WorkspacePage: "WorkspacePage" },
    "./WorkspaceRecents": { WorkspaceRecentRow: "WorkspaceRecentRow" },
    "./useWorkspaceRecents": { useWorkspaceRecents: () => recents },
  });
  const { WorkspaceRecentChatsSheet } = h.load(
    fileURLToPath(new URL("./WorkspaceRecentChatsSheet.tsx", import.meta.url)),
  ) as any;
  h.mount(() => WorkspaceRecentChatsSheet());
  return { h, opened };
}

test("Recent chats lists the sidebar's Recents rows with unread counts and opens the tapped thread", () => {
  const chat = { kind: "agent", agentId: "duo", conversationId: "nyxa-1" } as const;
  const crew = { kind: "group", groupId: "nyxg-1" } as const;
  const x = setup(
    { rows: [row(1, chat), row(2, crew)] },
    { "nyxa-1": 2 },
  );
  const page = x.h.value();
  assert.equal(page.type, "WorkspacePage");
  assert.equal(page.props.title, "Recent chats");
  const rows = nodes(page).filter((n) => n.type === "WorkspaceRecentRow");
  assert.deepEqual(rows.map((n) => n.props.item.title), ["Thread 1", "Thread 2"]);
  assert.deepEqual(rows.map((n) => n.props.unread), [2, 0]);
  rows[1]!.props.onSelect(crew);
  rows[0]!.props.onSelect(chat);
  assert.deepEqual(x.opened, [crew, chat]);
  assert.equal(nodes(page).some((n) => n.type === "SkeletonRows"), false);
  x.h.unmount();
});

test("Recent chats explains an empty, loading, offline or failed list", () => {
  const cases = [
    [{ loading: true }, "SkeletonRows"],
    [{}, "No recent chats."],
    [{ online: false }, "Offline · showing saved recent chats."],
    [{ error: true }, "Couldn’t load recent chats."],
  ] as const;
  for (const [data, expected] of cases) {
    const x = setup(data);
    const all = nodes(x.h.value());
    if (expected === "SkeletonRows")
      assert.ok(all.some((n) => n.type === "SkeletonRows"));
    else
      assert.ok(
        all.some((n) => n.type === "Text" && text(n) === expected),
        expected,
      );
    x.h.unmount();
  }
});
