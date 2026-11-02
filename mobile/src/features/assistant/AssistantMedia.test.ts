import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness, deferred } from "./assistantHookTestHarness";
import { ApiError } from "../../lib/api/ApiError";
import { activitySchema } from "./assistantTypes";
import { groupMessageSchema } from "../groups/groupContracts";
type E = { type: unknown; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const words = (n: any): string =>
  typeof n === "string" || typeof n === "number"
    ? String(n)
    : n?.props
      ? [n.props.children].flat(Infinity).map(words).join(" ")
      : "";
const jsx = {
  Fragment: "Fragment",
  jsx: (type: unknown, props: unknown) => ({ type, props }),
  jsxs: (type: unknown, props: unknown) => ({ type, props }),
};
function setup(
  file: string,
  name: string,
  props: any,
  mocks: Record<string, any> = {},
) {
  const h = hookHarness<E>({
    "react/jsx-runtime": jsx,
    "react-native": {
      View: "View",
      Pressable: "Pressable",
      StyleSheet: { create: (x: any) => x },
    },
    "../../components/workspace/WorkspaceUI": {
      Card: "Card",
      Label: "Label",
      ActionButton: "Button",
    },
    ...mocks,
  });
  const C = h.load(fileURLToPath(new URL(file, import.meta.url)))[name] as (
    p: any,
  ) => E;
  h.mount(() => C(props));
  return {
    h,
    text: () => words(h.value()),
    nodes: () => nodes(h.value()),
    press: (label: string) => {
      const n = nodes(h.value()).find(
        (n) => n.props.label === label || n.props.accessibilityLabel === label,
      );
      assert.ok(n, label);
      n.props.onPress();
      h.flush();
    },
  };
}
const at = "2026-10-28T00:00:00Z";
const activity = activitySchema.parse({
  id: "tool",
  label: "Read",
  status: "completed",
  started_at: at,
  machine: {
    node_id: "node",
    agent_id: "agent",
    operation_id: "op",
    job_id: "job",
    action: "command.run_shell",
    status: "completed",
    context_mode: "separated",
    exit_code: 0,
    bytes: 200,
    screenshot_id: "image",
    preview_id: "excerpt",
  },
});
test("real machine receipt renders correlation, metrics, screenshot and lazy excerpt/preference", async () => {
  let policyOptions: any;
  const calls: any[] = [];
  let refreshed = 0;
  const x = setup(
    "../machines/MachineToolCard.tsx",
    "MachineToolCard",
    {
      receipt: activity.machine,
      conversationId: "chat",
      attachments: [
        { id: "image", size: 1, content_type: "image/png", label: "Screen" },
      ],
      onOpenDesktop: () => {},
    },
    {
      "./MachineUI": { useMachineRuntime: () => ({ epoch: 1, active: true }) },
      "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
      "./MachineDesktopEntries": { MachineDesktopEntries: "Desktop" },
      "../assistant/AssistantAttachmentView": {
        AssistantAttachmentView: "Attachment",
      },
      "@tanstack/react-query": {
        useQuery: (o: any) => {
          policyOptions = o;
          return { data: { enabled: false }, refetch: async () => refreshed++ };
        },
      },
      "../../lib/api/http": {
        requestJson: async (...args: any[]) => {
          calls.push(args);
          return { enabled: true };
        },
      },
    },
  );
  assert.match(x.text(), /command · run shell/);
  assert.match(x.text(), /Exit.*0/);
  assert.match(x.text(), /200.*bytes/);
  assert.match(x.text(), /for this agent/);
  assert.equal(policyOptions.enabled, false);
  x.press("Correlation IDs");
  assert.match(x.text(), /Operation.*op/);
  assert.match(x.text(), /Job.*job/);
  assert.equal(x.nodes().filter((n) => n.type === "Attachment").length, 1);
  x.press("Show saved excerpt");
  assert.ok(
    x
      .nodes()
      .some(
        (n) =>
          typeof n.type === "function" &&
          n.props.endpoint?.endsWith("/attachments/excerpt"),
      ),
  );
  x.press("Command excerpt preference");
  assert.equal(policyOptions.enabled, true);
  x.press("Save future excerpts");
  await x.h.settle();
  assert.equal(calls[0][1].method, "PUT");
  assert.equal(calls[0][1].body.enabled, true);
  assert.equal(refreshed, 1);
  x.h.unmount();
});
test("real excerpt is authenticated, bounded, retention aware and cancels on close", async () => {
  for (const expired of [false, true]) {
    const pending = deferred<{ text: () => Promise<string> }>();
    let signal: AbortSignal | undefined;
    const x = setup(
      "../machines/MachineToolCard.tsx",
      "CommandExcerpt",
      { endpoint: "/private/excerpt", active: true },
      {
        "./MachineUI": {},
        "./MachineDesktopEntries": {},
        "../assistant/AssistantAttachmentView": {},
        "@tanstack/react-query": {},
        "../../lib/auth/authEpoch": { getAuthEpoch: () => 1 },
        "../../lib/api/http": {
          requestAuthenticatedResponse: (_path: string, o: any) => {
            signal = o.signal;
            return pending.promise;
          },
        },
      },
    );
    assert.match(x.text(), /Loading excerpt/);
    if (expired)
      pending.reject(
        new ApiError({
          errorKey: "expired",
          errorCode: 12101,
          statusCode: 410,
          message: "private",
        }),
      );
    else pending.resolve({ text: async () => "x".repeat(6000) });
    await x.h.settle();
    assert.equal(
      expired
        ? x.text().includes("Command excerpt expired per retention policy.")
        : x.text().trim().length === 4096,
      true,
    );
    x.h.unmount();
    assert.equal(signal?.aborted, true);
  }
});
test("crew schema retains typed machine/failure metadata for notice and agent rows", () => {
  for (const role of ["notice", "agent"] as const) {
    const row = groupMessageSchema.parse({
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      seq: 1,
      role,
      text: "Done",
      created_at: at,
      activities: [{ ...activity, failure: { kind: "tool_error" } }],
    });
    assert.equal(row.activities[0]?.machine?.job_id, "job");
    assert.equal(row.activities[0]?.failure?.kind, "tool_error");
  }
});
function imageHarness(captionColor?: string) {
  let epoch = 1,
    focused = true;
  let deleted = 0;
  let writes = 0;
  const requests: any[] = [];
  let background: (s: string) => void = () => {};
  const pending = deferred<Uint8Array>();
  class Dir {
    exists = true;
    create() {}
    delete() {
      deleted++;
      this.exists = false;
    }
  }
  class File {
    uri = "file:///private/image.png";
    create() {}
    write() {
      writes++;
    }
  }
  const props = {
    attachment: {
      id: "image",
      label: "Browser screen",
      size: 3,
      content_type: "image/png",
      origin: "tool",
      image_input: "unavailable",
    },
    conversationId: "chat",
    captionColor,
  };
  const x = setup(
    "./AssistantAttachmentView.tsx",
    "AssistantAttachmentView",
    props,
    {
      "react-native": {
        View: "View",
        Pressable: "Pressable",
        Image: "Image",
        Modal: "Modal",
        ActivityIndicator: "Spinner",
        StyleSheet: { create: (x: any) => x },
        AppState: {
          currentState: "active",
          addEventListener: (_e: string, fn: any) => {
            background = fn;
            return { remove() {} };
          },
        },
      },
      "@react-navigation/native": { useIsFocused: () => focused },
      "react-native-safe-area-context": { SafeAreaView: "Safe" },
      "expo-file-system": { Directory: Dir, File, Paths: { cache: "cache" } },
      "expo-sharing": {},
      "lucide-react-native": {
        Download: "Download",
        FileText: "File",
        ImageIcon: "ImageIcon",
        X: "X",
      },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../auth/AuthSessionContext": {
        useAuthSession: () => ({ isAuthenticated: true }),
      },
      "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
      "../../lib/api/http": {
        requestAuthenticatedBytes: (path: string, signal: AbortSignal) => {
          requests.push({ path, signal });
          return requests.length === 1
            ? pending.promise
            : new Promise<Uint8Array>(() => {});
        },
      },
    },
  );
  return {
    ...x,
    pending,
    requests,
    deleted: () => deleted,
    writes: () => writes,
    blur: () => {
      focused = false;
      x.h.render();
    },
    account: () => {
      epoch++;
      x.h.render();
    },
    background: () => {
      background("background");
      x.h.flush();
    },
  };
}
test("visible tool images use authenticated bytes inline, preserve the viewer, and clean private cache", async () => {
  const x = imageHarness();
  assert.equal(x.requests.length, 1);
  assert.match(x.requests[0].path, /conversations\/chat\/attachments\/image/);
  assert.match(x.text(), /Loading image from.*Browser screen/);
  x.pending.resolve(new Uint8Array([1, 2, 3]));
  await x.h.settle();
  assert.equal(x.writes(), 1);
  assert.ok(
    x
      .nodes()
      .some(
        (n) =>
          n.type === "Image" &&
          n.props.accessibilityLabel === "Image from Browser screen",
      ),
  );
  assert.match(x.text(), /agent couldn't view this image/);
  assert.equal(x.nodes().find((n) => n.type === "Modal")?.props.visible, false);
  x.press("Image from Browser screen");
  assert.equal(x.nodes().find((n) => n.type === "Modal")?.props.visible, true);
  x.press("Close attachment preview");
  assert.equal(x.deleted(), 0);
  x.background();
  assert.equal(x.deleted(), 1);
  x.h.unmount();
});
test("late inline image reads cannot write files after blur or account switch", async () => {
  for (const action of ["blur", "account"] as const) {
    const x = imageHarness();
    x[action]();
    assert.equal(x.requests[0].signal.aborted, true);
    x.pending.resolve(new Uint8Array([1, 2, 3]));
    await x.h.settle();
    assert.equal(x.writes(), 0);
    x.h.unmount();
  }
});

test("attachment captions on a user bubble retain the chosen foreground", () => {
  const x = imageHarness("#FFFFFF");
  const caption = x.nodes().find((node) => node.type === "Label" && words(node).includes("agent couldn't view"));
  assert.ok(caption);
  assert.equal(Object.assign({}, ...[caption.props.style].flat()).color, "#FFFFFF");
  x.h.unmount();
});

test("real crew receipt renderer supports notice/agent rows and removes duplicate screenshot attachments", () => {
  for (const role of ["notice", "agent"] as const) {
    const message = groupMessageSchema.parse({
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      seq: 1,
      role,
      text: "Done",
      created_at: at,
      activities: [activity],
      attachments: [
        { id: "image", label: "Screen", size: 3, content_type: "image/png" },
        { id: "doc", label: "Report", size: 3, content_type: "text/plain" },
      ],
    });
    const x = setup(
      "../groups/GroupMachineReceipts.tsx",
      "GroupMachineReceipts",
      { message, groupId: "crew" },
      {
        "../machines/MachineToolCard": { MachineToolCard: "Machine" },
        "../assistant/AssistantAttachmentView": {
          AssistantAttachmentView: "Attachment",
        },
      },
    );
    const machines = x.nodes().filter((n) => n.type === "Machine");
    const attachments = x.nodes().filter((n) => n.type === "Attachment");
    assert.equal(machines.length, 1);
    assert.equal(machines[0]!.props.groupId, "crew");
    assert.equal(attachments.length, 1);
    assert.equal(attachments[0]!.props.attachment.id, "doc");
    assert.equal(attachments[0]!.props.groupId, "crew");
    x.h.unmount();
  }
});
