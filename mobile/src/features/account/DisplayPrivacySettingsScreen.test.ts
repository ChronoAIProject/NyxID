import {translate} from "../../i18n/translate";
import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { buildApiError } from "../../lib/api/responseError";
import { elements, setupSettingsScreen, style } from "./settingsTestHarness";

test("Display previews appearance only in its Light/Dark tiles, with radios and the Automatic switch", () => {
  const env = setupSettingsScreen(
    "./DisplaySettingsScreen.tsx",
    "DisplaySettingsScreen",
  );
  const previews = env.all().filter((node) => node.type === "ChatPreview");
  assert.deepEqual(
    previews.map((node) => node.props.mode),
    ["light", "dark"],
    "no separate preview above the tiles",
  );
  const tile = (label: string) =>
    env.all().find((node) => node.props.accessibilityLabel === label)!;
  assert.equal(tile("Light appearance").props.accessibilityRole, "radio");
  assert.equal(tile("Light appearance").props.accessibilityState.checked, true);
  assert.equal(tile("Dark appearance").props.accessibilityState.checked, false);
  tile("Dark appearance").props.onPress();
  assert.deepEqual(env.preferences, ["dark"]);
  const automatic = elements(env.row("Automatic")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(automatic.props.value, true);
  automatic.props.onValueChange(false);
  assert.deepEqual(env.preferences, ["dark", "light"]);
  assert.ok(
    env.all().some((node) => node.type === "ChatDisplaySettings"),
    "bubble colour and chat layout render their own groups",
  );
  env.harness.unmount();
});

test("Privacy holds both analytics switches with the retention note", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
  );
  const account = elements(env.row("Allow for my account")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(
    account.props.accessibilityLabel,
    "Allow account usage analytics",
  );
  assert.equal(account.props.value, true);
  const device = elements(env.row("Allow on this device")).find(
    (node) => node.type === "Switch",
  )!;
  assert.equal(
    device.props.accessibilityLabel,
    "Allow usage analytics on this device",
  );
  assert.equal(device.props.value, false);
  assert.ok(
    env.text(
      "Optional usage events are sent only when both your account and this device allow them. Security and billing records have separate retention rules.",
    ),
  );
  env.harness.unmount();
});

test("Privacy exposes a first-party export row and disables it while sharing", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
  );
  const row = env.row("Export my data");
  assert.equal(row.props.disabled, false);
  assert.equal(row.props.accessibilityState.disabled, false);
  assert.ok(
    env.text(
      "Creates a bounded JSON export. You can request one up to three times per hour.",
    ),
  );
  env.pending();
  const pendingRow = env.row("Export my data");
  assert.equal(pendingRow.props.disabled, true);
  assert.equal(pendingRow.props.accessibilityState.busy, true);
  env.harness.unmount();
});

test("Privacy export result toasts request two lines without changing analytics toasts", () => {
  const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen");
  const [analytics, exportData] = env.mutationOptions;
  const toast = () => env.all().find((node) => node.type === "ToastOverlay")!.props.toast;
  try {
    exportData!.onSuccess();
    env.harness.render();
    assert.equal(toast().lines, 2);
    for (const error of [new Error("failed"), ...[404, 405, 429].map((statusCode) => new ApiError({
      errorKey: "export_failed", errorCode: 1000, statusCode, message: "failed", retryAfterMs: 130_000,
    }))]) {
      exportData!.onError(error);
      env.harness.render();
      assert.equal(toast().lines, 2);
    }
    analytics!.onError();
    env.harness.render();
    assert.equal(toast().lines, undefined);
  } finally {
    env.harness.unmount();
  }
});

test("Privacy analytics keeps shared group insets and wrapping controls at narrow widths and large text", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
  );
  for (const [width, fontScale] of [
    [402, 1],
    [320, 1],
    [402, 3.12],
  ]) {
    env.resize(width!, fontScale!);
    const title = env.text("USAGE ANALYTICS");
    assert.equal(style(title).textTransform, undefined);
    assert.equal(style(title).letterSpacing, 0.6);
    const footer = env.text(
      "Optional usage events are sent only when both your account and this device allow them. Security and billing records have separate retention rules.",
    );
    const group = env.parent(title);
    assert.equal(env.parent(footer), group);
    assert.ok(
      style(group).gap >= 8,
      "title and retention text have a stable gap from the card",
    );
    assert.equal(
      style(title).paddingHorizontal,
      style(footer).paddingHorizontal,
    );
    for (const label of ["Allow for my account", "Allow on this device"]) {
      const row = env.row(label);
      assert.equal(
        style(row).paddingHorizontal,
        style(title).paddingHorizontal,
      );
      const text = env.text(label);
      const labels = env.parent(env.parent(text));
      const body = env.parent(labels);
      assert.equal(style(labels).minWidth, 0);
      assert.equal(style(labels).flexShrink, 1);
      assert.equal(style(body).maxWidth, "100%");
      assert.equal(
        style(body).flexDirection,
        width! / fontScale! < 340 ? "column" : "row",
      );
      const control = elements(row).find((node) => node.type === "Switch")!;
      assert.equal(control.props.disabled, false);
      for (const node of [title, footer, text]) {
        assert.ok(
          style(node).textAlign === undefined ||
            style(node).textAlign === "left",
          "analytics text keeps the native left alignment",
        );
        assert.equal(node.props.numberOfLines, undefined);
        assert.notEqual(node.props.allowFontScaling, false);
        assert.equal(node.props.maxFontSizeMultiplier, undefined);
      }
    }
  }
  elements(env.row("Allow for my account"))
    .find((node) => node.type === "Switch")!
    .props.onValueChange(false);
  elements(env.row("Allow on this device"))
    .find((node) => node.type === "Switch")!
    .props.onValueChange(true);
  assert.deepEqual(env.mutations, [false]);
  assert.deepEqual(env.deviceChoices, [true]);
  env.harness.unmount();
});


test("Privacy export uses HTTP-date Retry-After diagnostics in its retry toast", (t) => {
  const now = Date.UTC(2030, 0, 1);
  t.mock.method(Date, "now", () => now);
  const error = buildApiError(
    { error: "rate_limited", error_code: 1005, message: "busy" },
    429,
    new Headers({ "Retry-After": new Date(now + 90_000).toUTCString() }),
  );
  assert.equal(error.retryAfterMs, 90_000);
  const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen");
  try {
    env.mutationOptions[1]!.onError(error);
    env.harness.render();
    const toast = env.all().find((node) => node.type === "ToastOverlay")!.props.toast;
    assert.equal(toast.message, "Your data export is temporarily rate limited. Try again in 2 minutes.");
  } finally {
    env.harness.unmount();
  }
});

test("Privacy export uses generic rate-limit wording for a zero Retry-After", () => {
  const error = buildApiError(
    { error: "rate_limited", error_code: 1005, message: "busy" },
    429,
    new Headers({ "Retry-After": "0" }),
  );
  assert.equal(error.retryAfterMs, 0);
  const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen");
  try {
    env.mutationOptions[1]!.onError(error);
    env.harness.render();
    const toast = env.all().find((node) => node.type === "ToastOverlay")!.props.toast;
    assert.equal(toast.message, "Your data export is temporarily rate limited.");
  } finally {
    env.harness.unmount();
  }
});

test("Privacy export uses neutral share wording and second-based busy retries", () => {
  for (const language of ["en", "zh-CN", "zh-TW"] as const) {
    const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen", { language });
    const exportData = env.mutationOptions[1]!;
    const toast = () => env.all().find((node) => node.type === "ToastOverlay")!.props.toast;
    exportData.onSuccess(); env.harness.render();
    assert.equal(toast().message, language === "en" ? "Export share sheet closed." : language === "zh-CN" ? "导出分享面板已关闭。" : "匯出分享面板已關閉。");
    assert.equal(toast().kind, "info");
    exportData.onError(new ApiError({errorKey:"busy", errorCode:1005, statusCode:429, message:"busy", retryAfterMs: 1_000}));
    env.harness.render();
    assert.match(toast().message, language === "en" ? /1 second/ : /1 秒/);
    env.harness.unmount();
  }
});

test("Privacy export watchdog failures use the existing localized error toast", () => {
  for (const language of ["en", "zh-CN", "zh-TW"] as const) {
    const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen", { language });
    try {
      for (const reason of ["export_headers_timeout", "export_idle_timeout", "export_deadline_exceeded"]) {
        env.mutationOptions[1]!.onError(new Error(reason));
        env.harness.render();
        const toast = env.all().find((node) => node.type === "ToastOverlay")!.props.toast;
        assert.equal(toast.kind, "error");
        assert.equal(toast.lines, 2);
        assert.equal(toast.message, language === "en"
          ? "We couldn't create your data export. Please try again."
          : language === "zh-CN" ? "无法创建数据导出，请重试。" : "無法建立資料匯出，請重試。");
      }
    } finally { env.harness.unmount(); }
  }
});

test("Privacy shows diagnostics status and explicitly limits Android coverage", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
    { diagnosticsOffered: true },
  );
  assert.ok(env.row("Crash diagnostics"));
  assert.ok(env.row("Review updated analytics"));
  assert.ok(env.text("Crash diagnostics need updated permission"));
  assert.ok(env.text("iOS includes native crashes and JavaScript errors. Android reports JavaScript errors and JavaScript fatal errors only; JVM and NDK crashes are not captured."));
  env.harness.unmount();
});

test("Privacy hides review action after diagnostics are enabled", () => {
  const env = setupSettingsScreen(
    "./PrivacySettingsScreen.tsx",
    "PrivacySettingsScreen",
    {
      diagnosticsOffered: true,
      diagnosticsAvailable: true,
      deviceEnabled: true,
      devicePolicy: 2,
    },
  );
  assert.ok(env.row("Crash diagnostics"));
  assert.equal(
    env.all().find(
      (node) =>
        node.type === "Text" &&
        [node.props.children].flat(Infinity).join("") ===
          "Review updated analytics",
    ),
    undefined,
  );
  assert.ok(env.text("Crash diagnostics enabled"));
  env.harness.unmount();
});

test("Privacy hides all policy-2 groups and rows when diagnostics is not offered", () => {
  const env=setupSettingsScreen("./PrivacySettingsScreen.tsx","PrivacySettingsScreen", {diagnosticsOffered:false});
  const text=env.all().map(node=>node.props.children).flat(Infinity);
  assert.ok(!text.includes("CRASH DIAGNOSTICS"));
  assert.ok(!text.includes("Crash diagnostics"));
  assert.ok(!text.includes("Review updated analytics"));
  assert.ok(env.row("Allow for my account"));
  assert.ok(env.row("Allow on this device"));
  env.harness.unmount();
});

for (const [accountPolicy,devicePolicy,offered,expected] of [["2",1,true,2],["1",2,true,2],["2",2,false,1],["1",1,true,1]] as const) test(`account switch keeps acknowledged policy account=${accountPolicy} device=${devicePolicy} offered=${offered}`,()=>{
  const env=setupSettingsScreen("./PrivacySettingsScreen.tsx","PrivacySettingsScreen",{diagnosticsOffered:offered,accountPolicy,devicePolicy});
  env.mutationOptions[0]!.mutationFn(true);
  assert.equal(env.analyticsSaves[0]![5],expected);
  env.harness.unmount();
});

for (const [language, reviewMessage] of [
  ["en", "Could not save updated analytics permission."],
  ["zh-CN", "无法保存更新后的分析授权。"],
  ["zh-TW", "無法儲存更新後的分析授權。"],
] as const) {
  test(`Privacy policy-1 failure feedback follows app language (${language})`, () => {
    const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen", {
      diagnosticsOffered: false, accountPolicy: "1", devicePolicy: 1, language,
    });
    env.mutationOptions[0]!.onError!();
    env.harness.flush();
    const toast = env.all().find((node) => node.type === "ToastOverlay");
    assert.equal(toast?.props.toast?.message,
      translate(language, "Could not save account analytics permission. This device remains off; retry to withdraw across devices."));
    env.harness.unmount();
  });
  test(`Privacy policy-2 review failure feedback is localized (${language})`, () => {
    const env = setupSettingsScreen("./PrivacySettingsScreen.tsx", "PrivacySettingsScreen", {
      diagnosticsOffered: true, language,
    });
    env.mutationOptions[2]!.onError!();
    env.harness.flush();
    const toast = env.all().find((node) => node.type === "ToastOverlay");
    assert.equal(toast?.props.toast?.message, reviewMessage);
    env.harness.unmount();
  });
}
