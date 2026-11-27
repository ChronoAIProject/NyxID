import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "./assistantHookTestHarness";
import { QueryClient, QueryObserver, environmentManager, onlineManager } from "@tanstack/query-core";
import { buildApiError } from "../../lib/api/responseError";
import { COMPOSER_ACCESS_DESCRIPTION, composerSettingsRecoveryInterval, scheduleComposerSettingsRecovery } from "./composerSettings";

type Element = { type: unknown; props: Record<string, any> };
const nodes = (n: any): Element[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];
const plain = (v: any) => JSON.parse(JSON.stringify(v));
function setup(theme = "light", cornerWindow = false) {
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
    "../../components/workspace/WorkspaceLayoutContext": {
      WorkspaceCornerSensorContext: { value: cornerWindow },
    },
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
      ActivityIndicator: "Spinner",
      StyleSheet: { absoluteFill: { position: "absolute" }, hairlineWidth: 0.5, },
    },
    "lucide-react-native": {
      Check: "Check",
      ChevronDown: "ChevronDown",
      X: "X",
    },
    "../../components/workspace/WorkspaceUI": { Label: "Label" },
    "../../theme/ThemeContext": {
      useTheme: () => ({
        preference: theme,
        mode: theme === "system" ? "dark" : theme,
        colors: {
          card: "surface",
          sheetScrim: "scrim",
          composerBg: "composer",
          composerBorder: "composer-border",
          shadowColor: "shadow",
        },
      }),
    },
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
  h.mock("./useComposerSettingsQuery", h.load(
    fileURLToPath(new URL("./useComposerSettingsQuery.ts", import.meta.url)),
  ));
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
    open: () => {
      find("Permissions: Auto").props.onPress();
      h.render();
      assert.equal(props.visible, true);
    },
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

test("the Duo mode chip is a plain pill with the current mode only: no chevron, same picker, hint and 44pt target", async () => {
  for (const theme of ["light", "dark"]) {
    const x = setup(theme, true);
    const trigger = x.find("Permissions: Auto");
    const inside = nodes(trigger);
    assert.equal(inside.some((n) => n.type === "ChevronDown"), false,);
    assert.equal(trigger.props.accessibilityHint, "Changes the permission mode",);
    const style = (pressed: boolean) => trigger.props.style({ pressed });
    assert.equal(style(false).minHeight, 44);
    assert.equal(style(false).minWidth, 44);
    assert.equal(style(false).opacity, 1);
    assert.ok(style(true).opacity < 1, "pressed state");
    const pill = inside.find((n) => n.props.testID === "composer-access-mode-pill",)!;
    const pillStyle = Object.assign({}, ...[pill.props.style].flat());
    // The floating surface the header controls and the composer use.
    assert.equal(pillStyle.backgroundColor, "composer");
    assert.equal(pillStyle.borderColor, "composer-border");
    assert.equal(pillStyle.borderWidth, 0.5);
    assert.equal(pillStyle.shadowRadius, theme === "light" ? 10 : undefined);
    assert.equal(pillStyle.borderRadius, 15);
    const label = inside.find((n) => n.type === "Label")!;
    assert.equal(label.props.children, "Auto");
    assert.equal(label.props.numberOfLines, 1);
    assert.equal(label.props.ellipsizeMode, "tail");
    // Same existing picker, opened by the same press.
    x.open();
    x.h.unmount();
  }
  const { setAppTranslationLanguage, translateApp } = await import(
    "../../i18n/appTranslation"
  );
  try {
    for (const language of ["zh-CN", "zh-TW"] as const) {
      setAppTranslationLanguage(language);
      assert.notEqual(
        translateApp("Changes the permission mode"),
        "Changes the permission mode",
        language,
      );
    }
  } finally {
    setAppTranslationLanguage("en");
  }
});

test("the Duo pill keeps every current mode and the base explicit-retry policy", () => {
  const x = setup("light", true);
  for (const [mode, label] of [
    ["autopilot", "Full access"],
    ["nyxmode", "Auto"],
    ["ask_every_time", "Ask every time"],
  ] as const) {
    x.settings.data = { access_mode: mode };
    x.h.render();
    const trigger = x.find(`Permissions: ${label}`);
    const visible = nodes(trigger).find((n) => n.type === "Label")!;
    assert.equal(visible.props.children, label, mode);
    assert.equal(
      trigger.props.accessibilityHint,
      "Changes the permission mode",
    );
  }
  const query = x.queries.at(-1);
  assert.equal(query.retry, false);
  assert.equal(query.enabled({ state: { status: "error" } }), false);
  x.h.unmount();
});

test("offline cold start recovers permissions after the first connected GET still fails", { timeout: 10_000 }, async (t) => {
  const x = setup();
  const options = x.queries.at(-1);
  const wasServer = environmentManager.isServer();
  environmentManager.setIsServer(() => false);
  const client = new QueryClient();
  const originalOnline = onlineManager.isOnline();
  onlineManager.setOnline(false);
  client.mount();
  let attempts = 0;
  const observer = new QueryObserver(client, {
    ...options,
    queryFn: async () => {
      if (++attempts === 1) throw buildApiError(null, 0);
      return { access_mode: "nyxmode" };
    },
  });
  const done = deferred<void>();
  const unsubscribe = observer.subscribe((result) => {
    if (result.isSuccess) done.resolve();
    if (result.isError) {
      const query = client.getQueryCache().find({ queryKey: options.queryKey })!;
      const delay = composerSettingsRecoveryInterval(query);
      if (delay !== false)
        scheduleComposerSettingsRecovery(result.errorUpdatedAt, delay,
          () => void observer.refetch({ cancelRefetch: false }));
    }
  });
  t.after(() => {
    unsubscribe();
    client.unmount();
    client.clear();
    onlineManager.setOnline(originalOnline);
    environmentManager.setIsServer(() => wasServer);
    x.h.unmount();
  });
  assert.equal(observer.getCurrentResult().fetchStatus, "paused");
  assert.equal(attempts, 0);
  onlineManager.setOnline(true);
  await done.promise;
  assert.deepEqual(observer.getCurrentResult().data, { access_mode: "nyxmode" });
  assert.equal(attempts, 2);
  assert.equal(composerSettingsRecoveryInterval(client.getQueryCache().find({ queryKey: options.queryKey })!), false);
});

test("permission recovery backs off on network or gateway failures and leaves other errors for manual retry", () => {
  for (const [code, expected] of [[0, 5000], [503, 5000], [403, false], [429, 5000]] as const) {
    assert.equal(composerSettingsRecoveryInterval({
      state: { status: "error", error: buildApiError(null, code), errorUpdateCount: 1 },
    }), expected);
  }
  assert.equal(composerSettingsRecoveryInterval({
    state: { status: "error", error: buildApiError(null, 0), errorUpdateCount: 10 },
  }), 30_000);
});

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
  x.open();
  x.writeMode(async () => {
    throw new Error("Permission policy changed.");
  });
  x.find("Full access").props.onPress();
  await x.h.settle();
  assert.equal(x.find("Auto").props.accessibilityState.checked, true);
  assert.equal(x.props.visible, true);
  assert.ok(
    nodes(x.h.value()).some(
      (n) =>
        n.props.accessibilityRole === "alert" &&
        n.props.children === "Could not change permissions.",
    ),
  );
  x.h.unmount();
});

test("permissions automatically close after a confirmed write in every appearance", async () => {
  for (const theme of ["light", "dark", "system"]) {
    const x = setup(theme, true);
    x.open();
    const wait = deferred<any>();
    x.writeMode(() => wait.promise);
    const change = x.find("Full access").props.onPress;
    change();
    change();
    x.h.flush();
    assert.equal(x.props.visible, true);
    assert.equal(x.find("Full access").props.disabled, true);
    assert.ok(
      nodes(x.h.value()).some(
        (n) => n.props.children === "Saving permissions…",
      ),
    );
    wait.resolve({ access_mode: "autopilot" });
    await x.h.settle();
    assert.equal(x.props.visible, false);
    assert.ok(x.find("Permissions: Full access"));
    assert.deepEqual(
      x.calls.filter((c) => c[0] === "PUT"),
      [["PUT", "autopilot"]],
    );
    assert.ok(
      x.calls.findIndex((c) => c[0] === "cache") <
        x.calls.findLastIndex((c) => c[0] === "open"),
    );
    x.h.unmount();
  }
});

test("a stalled permissions save stays dismissible and reopening preserves its write lock until settlement", async () => {
  for (const action of ["backdrop", "close", "back", "escape"]) {
    const x = setup();
    x.open();
    const wait = deferred<any>();
    x.writeMode(() => wait.promise);
    const change = x.find("Full access").props.onPress;
    change();
    x.h.flush();
    // No response or timeout has settled this request when the user leaves.
    if (action === "backdrop") x.find("Dismiss permissions").props.onPress();
    if (action === "close") x.find("Close permissions").props.onPress();
    if (action === "back")
      nodes(x.h.value())
        .find((n) => n.type === "Modal")!
        .props.onRequestClose();
    if (action === "escape")
      nodes(x.h.value())
        .find((n) => n.props.accessibilityViewIsModal)!
        .props.onAccessibilityEscape();
    assert.equal(x.props.visible, false, action);
    x.open();
    assert.equal(x.find("Full access").props.disabled, true);
    assert.equal(x.find("Dismiss permissions").props.disabled, undefined);
    assert.equal(x.find("Close permissions").props.disabled, undefined);
    assert.ok(
      nodes(x.h.value()).some(
        (n) => n.props.children === "Saving permissions…",
      ),
    );
    change();
    assert.deepEqual(
      x.calls.filter((c) => c[0] === "PUT"),
      [["PUT", "autopilot"]],
    );
    wait.resolve({ access_mode: "autopilot" });
    await x.h.settle();
    assert.equal(x.props.visible, false);
    assert.ok(x.find("Permissions: Full access"));
    x.h.unmount();
  }
});

test("a permissions save settling after dismissal does not close another shared sheet", async () => {
  const x = setup();
  let sheet: "permissions" | "agents" | undefined;
  x.props.onOpenChange = (open: boolean) => {
    sheet = open ? "permissions" : undefined;
    x.props.visible = sheet === "permissions";
  };
  x.h.render();
  x.open();
  assert.equal(sheet, "permissions");
  const wait = deferred<any>();
  x.writeMode(() => wait.promise);
  x.find("Full access").props.onPress();
  x.h.flush();
  x.find("Dismiss permissions").props.onPress();
  x.h.render();
  assert.equal(sheet, undefined);
  sheet = "agents";
  x.props.visible = false;
  x.h.render();

  wait.resolve({ access_mode: "autopilot" });
  await x.h.settle();
  assert.equal(x.settings.data.access_mode, "autopilot");
  assert.equal(x.calls.filter((call) => call[0] === "cache").length, 1);
  assert.equal(sheet, "agents");
  x.h.unmount();
});

test("backdrop, close, system Back and accessibility Escape dismiss without a setting write", () => {
  for (const action of ["backdrop", "close", "back", "escape"]) {
    const x = setup();
    x.open();
    if (action === "backdrop") x.find("Dismiss permissions").props.onPress();
    if (action === "close") x.find("Close permissions").props.onPress();
    if (action === "back")
      nodes(x.h.value())
        .find((n) => n.type === "Modal")!
        .props.onRequestClose();
    if (action === "escape")
      nodes(x.h.value())
        .find((n) => n.props.accessibilityViewIsModal)!
        .props.onAccessibilityEscape();
    assert.equal(x.props.visible, false, action);
    assert.equal(
      x.calls.some((c) => c[0] === "PUT"),
      false,
    );
    // Backdrop is a sibling of the card, so content touches cannot bubble into it.
    const backdrop = x.find("Dismiss permissions");
    assert.equal(
      nodes(backdrop).some((n) => n.props.accessibilityLabel === "Full access"),
      false,
    );
    x.h.unmount();
  }
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

test("P9 ordinary composer retains the labelled chevron trigger and base read policy", () => {
  const x = setup();
  const trigger = x.find("Permissions: Auto");
  assert.equal(
    nodes(trigger).some((n) => n.type === "ChevronDown"),
    true,
  );
  assert.equal(trigger.props.style.minHeight, 44);
  assert.equal(trigger.props.style.maxWidth, 180);
  assert.equal(
    nodes(trigger).find((n) => n.type === "Label")!.props.maxFontSizeMultiplier,
    1.3,
  );
  assert.equal(x.queries.at(-1).retry, false);
  assert.equal(x.queries.at(-1).refetchOnReconnect, undefined);
  x.h.unmount();
});
