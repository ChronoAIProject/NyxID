import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";

type Element = { type: any; props: Record<string, any> };
type AlertCall = {
  title: string;
  buttons?: Array<{ text: string; onPress?: () => void }>;
};

function setup(reported = false, fail = false) {
  const alerts: AlertCall[] = [];
  const reports: unknown[][] = [];
  let pending: ReturnType<typeof deferred<void>> | undefined;
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Alert: {
        alert: (
          title: string,
          _message: string,
          buttons?: AlertCall["buttons"],
        ) => alerts.push({ title, buttons }),
      },
      Pressable: "Pressable",
      View: "View",
      StyleSheet: { create: (value: unknown) => value },
    },
    "lucide-react-native": { Flag: "Flag" },
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { textMuted: "muted", danger: "red" } }),
    },
    "../../lib/api/replyReportApi": {
      replyReportApi: {
        report: async (...args: unknown[]) => {
          reports.push(args);
          if (pending) await pending.promise;
          if (fail) throw new Error("offline");
        },
      },
    },
    "../../lib/api/replyReportTransport": {
      REPORT_REASONS: [
        "Offensive or abusive",
        "Harmful or unsafe",
        "Inaccurate or misleading",
        "Something else",
      ],
    },
  });
  const { AssistantReportButton } = harness.load(
    fileURLToPath(new URL("./AssistantReportButton.tsx", import.meta.url)),
  ) as any;
  const target = { kind: "conversation", id: "c", messageId: "m" };
  harness.mount(() => AssistantReportButton({ target, reported }));
  return {
    harness,
    alerts,
    reports,
    target,
    button: () => harness.flush(),
    hold: () => {
      pending = deferred<void>();
      return pending;
    },
  };
}

test("choosing a reason sends one report and marks the reply reported", async () => {
  const env = setup();
  assert.equal(env.button().props.accessibilityLabel, "Report response");
  env.button().props.onPress();
  const picker = env.alerts[0]!;
  assert.equal(picker.title, "Report this response?");
  assert.deepEqual(
    JSON.parse(JSON.stringify(picker.buttons!.map((b) => b.text))),
    [
      "Offensive or abusive",
      "Harmful or unsafe",
      "Inaccurate or misleading",
      "Something else",
      "Cancel",
    ],
  );
  picker.buttons![0]!.onPress!();
  await env.harness.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(env.reports)), [
    [env.target, "Offensive or abusive"],
  ]);
  assert.equal(env.alerts.at(-1)!.title, "Report sent");
  assert.equal(env.button().props.accessibilityLabel, "Reported response");
});

test("cancelling sends nothing, and a pending report blocks a second picker", async () => {
  const env = setup();
  env.button().props.onPress();
  env.alerts[0]!.buttons!.at(-1)!.onPress?.();
  assert.equal(env.reports.length, 0);
  const held = env.hold();
  env.button().props.onPress();
  env.alerts[1]!.buttons![1]!.onPress!();
  env.harness.flush();
  assert.equal(env.button().props.disabled, true);
  env.button().props.onPress();
  assert.equal(env.alerts.length, 2);
  held.resolve();
  await env.harness.settle();
  assert.equal(env.reports.length, 1);
});

test("a failed report keeps the reply unreported and offers the support email", async () => {
  const env = setup(false, true);
  env.button().props.onPress();
  env.alerts[0]!.buttons![1]!.onPress!();
  await env.harness.settle();
  assert.equal(env.alerts.at(-1)!.title, "Couldn't send report");
  assert.equal(env.button().props.accessibilityLabel, "Report response");
});

test("a reply already reported on the server starts in the reported state", () => {
  const env = setup(true);
  assert.equal(env.button().props.accessibilityLabel, "Reported response");
});
