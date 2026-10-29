import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { COMPOSER_ACCESS_DESCRIPTION } from "./composerSettings";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (n: any): Element[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const plain = (v: any) => JSON.parse(JSON.stringify(v));
function setup() {
  let epoch = 1;
  let available = true;
  const calls: unknown[][] = [];
  const settings: any = {
    data: { access_mode: "nyxmode" },
    isPending: false,
    isError: false,
    refetch: () => calls.push(["retry-settings"]),
  };
  let modeWrite = async (mode: string) => ({ access_mode: mode });
  const queries: any[] = [];
  const props: any = {
    scope: "one",
    activeTurn: false,
    disabled: false,
    visible: false,
    onOpenChange: (open: boolean) => {
      props.visible = open;
      calls.push(["open", open]);
    },
  };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      Fragment: "Fragment",
      jsx: (type: unknown, props: any) => ({ type, props }),
      jsxs: (type: unknown, props: any) => ({ type, props }),
    },
    "react-native": {
      Modal: "Modal",
      Pressable: "Pressable",
      ScrollView: "ScrollView",
      View: "View",
    },
    "lucide-react-native": {
      Check: "Check",
      ChevronDown: "ChevronDown",
      X: "X",
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../lib/auth/authEpoch": { getAuthEpoch: () => epoch },
    "../groups/useGroupRuntime": {
      useGroupAvailability: () => ({
        active: available,
        foreground: available,
        focused: available,
        available: () => available,
      }),
    },
    "@tanstack/react-query": {
      useQuery: (options: any) => {
        queries.push(options);
        return settings;
      },
      useQueryClient: () => ({
        setQueryData: (key: any, value: any) => {
          calls.push(["cache", plain(key), plain(value)]);
          settings.data = value;
        },
      }),
    },
    "../../lib/api/assistantApi": {
      assistantApi: {
        settings: async () => settings.data,
        updateAccessMode: (mode: string) => {
          calls.push(["PUT", mode]);
          return modeWrite(mode);
        },
      },
    },
  });
  const C = h.load(
    fileURLToPath(new URL("./AssistantComposerSettings.tsx", import.meta.url)),
  ).AssistantComposerSettings as (props: any) => Element;
  h.mount(() => C(props));
  const find = (label: string) => {
    const row = nodes(h.value()).find(
      (n) => n.props.accessibilityLabel === label,
    );
    assert.ok(row, label);
    return row;
  };
  return {
    h,
    props,
    calls,
    settings,
    queries,
    find,
    writeMode: (write: typeof modeWrite) => {
      modeWrite = write;
    },
    available: (value: boolean) => {
      available = value;
      h.render();
    },
    account: () => {
      epoch++;
      h.render();
    },
  };
}

test("native settings expose only permissions and the three descriptions, with no discovery or model transport", () => {
  const x = setup();
  const text = nodes(x.h.value())
    .map((n) => (typeof n.props.children === "string" ? n.props.children : ""))
    .join(" ");
  assert.doesNotMatch(text, /model|thinking|research/i);
  for (const description of Object.values(COMPOSER_ACCESS_DESCRIPTION))
    assert.ok(text.includes(description));
  assert.ok(x.queries.every((q) => q.queryKey[1] === "composer-settings"));
  x.h.unmount();
});

test("Full access changes owner settings directly like web, once per pending write, with all three exact labels", async () => {
  const x = setup();
  const wait = deferred<any>();
  x.writeMode(() => wait.promise);
  for (const label of ["Full access", "Auto", "Ask every time"])
    assert.ok(x.find(label));
  const button = x.find("Full access");
  const first = button.props.onPress();
  const second = button.props.onPress();
  x.h.flush();
  assert.deepEqual(
    x.calls.filter((c) => c[0] === "PUT"),
    [["PUT", "autopilot"]],
  );
  assert.equal(x.find("Ask every time").props.disabled, true);
  wait.resolve({ access_mode: "autopilot" });
  await Promise.all([first, second]);
  await x.h.settle();
  assert.equal(x.find("Full access").props.accessibilityState.checked, true);
  assert.equal(x.find("Permissions: Full access").props.disabled, false);
  assert.equal(x.find("Full access").props.style.minHeight, 44);
  x.h.unmount();
});

test("agent overrides remain displayed and read-only even to a retained formerly editable callback", async () => {
  const x = setup();
  const stale = x.find("Full access").props.onPress;
  x.props.agentOverride = "ask_every_time";
  x.h.render();
  assert.ok(x.find("Permissions: Ask every time (this agent)"));
  for (const mode of ["Full access", "Auto", "Ask every time"])
    assert.equal(x.find(mode).props.disabled, true);
  await stale();
  assert.equal(
    x.calls.some((c) => c[0] === "PUT"),
    false,
  );
  x.h.unmount();
});

test("active, unavailable and missing-settings states refuse access changes; retries remain explicit", async () => {
  for (const reason of ["turn", "disabled", "offline", "loading"]) {
    const x = setup();
    const stale = x.find("Full access").props.onPress;
    if (reason === "turn") x.props.activeTurn = true;
    if (reason === "disabled") x.props.disabled = true;
    if (reason === "offline") x.available(false);
    if (reason === "loading") {
      x.settings.data = undefined;
      x.settings.isError = true;
    }
    x.h.render();
    await stale();
    assert.equal(
      x.calls.some((c) => c[0] === "PUT"),
      false,
      reason,
    );
    if (reason === "loading") {
      const retry = nodes(x.h.value()).find(
        (n) =>
          n.type === "Pressable" &&
          nodes(n).some(
            (t) => t.props.children === "Retry loading permissions",
          ),
      )!;
      retry.props.onPress();
      assert.ok(x.calls.some((c) => c[0] === "retry-settings"));
    }
    x.h.unmount();
  }
});

test("late permission responses cannot update another scope, account or unmounted composer", async () => {
  for (const fence of ["scope", "account", "unmount"]) {
    const x = setup();
    const wait = deferred<any>();
    x.writeMode(() => wait.promise);
    x.find("Full access").props.onPress();
    x.h.flush();
    if (fence === "scope") {
      x.props.scope = "other";
      x.h.render();
    }
    if (fence === "account") x.account();
    if (fence === "unmount") x.h.unmount();
    wait.resolve({ access_mode: "autopilot" });
    await x.h.settle();
    assert.equal(
      x.calls.some((c) => c[0] === "cache"),
      false,
      fence,
    );
    x.h.unmount();
  }
});

test("settings write failures retain authoritative mode and expose the error", async () => {
  const x = setup();
  x.writeMode(async () => {
    throw new Error("Permission policy changed.");
  });
  x.find("Full access").props.onPress();
  await x.h.settle();
  assert.equal(x.find("Auto").props.accessibilityState.checked, true);
  assert.ok(
    nodes(x.h.value()).some(
      (n) =>
        n.props.accessibilityRole === "alert" &&
        n.props.children === "Could not change permissions.",
    ),
  );
  x.h.unmount();
});

test("access-mode API posts only validated owner access_mode to the existing settings endpoint", async () => {
  const calls: any[] = [];
  const h = hookHarness<any>({
    "./http": {
      requestJson: async (path: string, options: any) => {
        calls.push([path, options]);
        return { access_mode: "autopilot" };
      },
    },
  });
  const api = h.load(
    fileURLToPath(new URL("../../lib/api/assistantApi.ts", import.meta.url)),
  ).assistantApi as any;
  await api.settings();
  await api.updateAccessMode("autopilot");
  assert.deepEqual(plain(calls[1]), [
    "/assistant/nyxagent/settings",
    { method: "PUT", body: { access_mode: "autopilot" } },
  ]);
  await assert.rejects(api.updateAccessMode("invalid"));
  assert.equal(calls.length, 2);
});
