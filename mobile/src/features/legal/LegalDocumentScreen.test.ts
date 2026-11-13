import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { deferred, hookHarness } from "../assistant/assistantHookTestHarness";
import * as legalLink from "./legalLink";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";

type Element = { type: unknown; props: Record<string, any> };
const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
const flatten = (styles: any): Record<string, any> =>
  Array.isArray(styles)
    ? Object.assign({}, ...styles.map(flatten))
    : styles || {};

function elements(root: Element): Element[] {
  return [
    root,
    ...[root.props.children]
      .flat(Infinity)
      .flatMap((child) =>
        child && typeof child === "object" && "props" in child
          ? elements(child)
          : [],
      ),
  ];
}

test("root legal documents share a readable column with their sticky Back control; sheets keep their own chrome", () => {
  for (const inSheet of [false, true]) {
    let backs = 0;
    const harness = hookHarness<Element>({
      "react/jsx-runtime": { jsx, jsxs: jsx },
      "react-native": {
        View: "View",
        Text: "Text",
        ScrollView: "ScrollView",
        StyleSheet: { create: <T>(styles: T) => styles },
      },
      "react-native-markdown-display": "Markdown",
      "../../app/WorkspaceHeader": {
        useWorkspaceSheet: () => (inSheet ? {} : null),
      },
      "../../components/BlurBackButton": { BlurBackButton: "BlurBackButton" },
      "../../components/ScreenContainer": {
        ScreenContainer: "ScreenContainer",
      },
      "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../theme/flowStyles": {
        createFlowStyles: () => ({ content: { flex: 1 } }),
      },
      "../../lib/telemetry": { capture: () => {} },
      "./legalDocumentUrl": { resolveLegalDocumentUrl: () => null },
    });
    const { LegalDocumentScreen } = harness.load(
      fileURLToPath(new URL("./LegalDocumentScreen.tsx", import.meta.url)),
    ) as { LegalDocumentScreen: (props: unknown) => Element };
    harness.mount(() =>
      LegalDocumentScreen({
        title: "Terms of Service",
        docKey: "terms",
        telemetryBackTarget: "auth",
        onBack: () => {
          backs++;
        },
        onOpenDocument: () => {},
      }),
    );
    const nodes = elements(harness.value());
    const scroller = nodes.find((item) => item.type === "ScrollView")!;
    const content = flatten(scroller.props.contentContainerStyle);
    assert.equal(
      flatten(scroller.props.style).flex,
      1,
      "the scrolling viewport still fills the screen",
    );
    assert.equal(content.maxWidth, inSheet ? undefined : 720);
    assert.equal(content.width, inSheet ? undefined : "100%");
    assert.equal(content.alignSelf, inSheet ? undefined : "center");
    const back = nodes.find((item) => item.type === "BlurBackButton");
    assert.equal(Boolean(back), !inSheet);
    if (back) {
      const column = nodes.find((item) => item.props.children === back)!;
      assert.equal(flatten(column.props.style).maxWidth, content.maxWidth);
      assert.equal(
        flatten(column.props.style).paddingHorizontal,
        content.paddingHorizontal,
      );
      back.props.onPress();
      assert.equal(backs, 1);
    }
    harness.unmount();
  }
});

type AlertButton = { text: string; onPress?: () => void; style?: string };

function documentFixture(
  docKey: legalLink.LegalDocumentKey,
  { inSheet = false, language = "en" as Language } = {},
) {
  const opened: string[] = [];
  const destinations: legalLink.LegalDocumentKey[] = [];
  const alerts: Array<{
    title: string;
    message: string;
    buttons: AlertButton[];
  }> = [];
  let openURL = async (_url: string) => {};
  let fetches = 0;
  let backs = 0;
  const h = hookHarness<Element>(
    {
      "react/jsx-runtime": { jsx, jsxs: jsx },
      "react-native": {
        View: "View",
        Text: "Text",
        ScrollView: "ScrollView",
        StyleSheet: { create: <T>(styles: T) => styles },
        Linking: {
          openURL: (url: string) => {
            opened.push(url);
            return openURL(url);
          },
        },
        Alert: {
          alert: (title: string, message: string, buttons: AlertButton[]) =>
            alerts.push({ title, message, buttons }),
        },
      },
      "react-native-markdown-display": "Markdown",
      "../../app/WorkspaceHeader": {
        useWorkspaceSheet: () => (inSheet ? {} : null),
      },
      "../../components/BlurBackButton": { BlurBackButton: "BlurBackButton" },
      "../../components/ScreenContainer": {
        ScreenContainer: "ScreenContainer",
      },
      "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../theme/flowStyles": {
        createFlowStyles: () => ({ content: { flex: 1 } }),
      },
      "../../lib/telemetry": { capture: () => {} },
      "../../i18n/LanguageProvider": {
        useAppLanguage: () => ({
          t: (phrase: string) => translate(language, phrase),
        }),
      },
      "./legalDocumentUrl": {
        resolveLegalDocumentUrl: () =>
          `https://nyxid.example/legal/${docKey}.md`,
      },
      "./legalLink": legalLink,
    },
    {
      fetch: async () => {
        fetches++;
        return {
          ok: true,
          text: async () =>
            readFileSync(
              new URL(
                `../../../../frontend/public/legal/${docKey}.md`,
                import.meta.url,
              ),
              "utf8",
            ),
        };
      },
    },
  );
  const { LegalDocumentScreen } = h.load(
    fileURLToPath(new URL("./LegalDocumentScreen.tsx", import.meta.url)),
  ) as { LegalDocumentScreen(props: unknown): Element };
  h.mount(() =>
    LegalDocumentScreen({
      title: docKey === "privacy" ? "Privacy Policy" : "Terms of Service",
      docKey,
      telemetryBackTarget: "settings",
      onBack: () => {
        backs++;
      },
      onOpenDocument: (target: legalLink.LegalDocumentKey) => {
        destinations.push(target);
      },
    }),
  );
  const markdown = () => {
    const node = elements(h.value()).find((node) => node.type === "Markdown");
    assert.ok(node, "expected a loaded legal document");
    return node;
  };
  return {
    h,
    markdown,
    opened,
    destinations,
    alerts,
    fetches: () => fetches,
    backs: () => backs,
    setOpenURL: (next: typeof openURL) => {
      openURL = next;
    },
  };
}

for (const inSheet of [false, true]) {
  for (const docKey of ["privacy", "terms"] as const) {
    test(`${docKey} canonical cross-page links navigate within the app and retain the source document (${inSheet ? "sheet" : "root"})`, async (t) => {
      const f = documentFixture(docKey, { inSheet });
      t.after(f.h.unmount);
      await f.h.settle();
      const originalBody = f.markdown().props.children;
      const target = docKey === "privacy" ? "terms" : "privacy";
      assert.match(
        originalBody,
        new RegExp(`\\]\\(/${target}\\)`),
        "exercise a real link from the published document",
      );
      assert.equal(
        f.markdown().props.onLinkPress(`/${target}`),
        false,
        "relative links must not fall through to Linking.openURL",
      );
      await f.h.settle();
      assert.deepEqual(f.destinations, [target]);
      assert.deepEqual(f.opened, []);
      assert.equal(
        f.markdown().props.children,
        originalBody,
        "the mounted source document remains intact for Back",
      );
      assert.equal(
        f.fetches(),
        1,
        "pushing a destination does not refetch or replace the source document",
      );
      assert.equal(f.alerts.length, 0);
      if (!inSheet) {
        const back = elements(f.h.value()).find(
          (node) => node.type === "BlurBackButton",
        );
        assert.ok(back);
        back.props.onPress();
        assert.equal(f.backs(), 1);
      }
    });
  }
}

test("native legal wrappers push the opposite document and pop on Back", () => {
  const pushes: string[] = [];
  let backs = 0;
  const h = hookHarness<Element>({
    "react/jsx-runtime": { jsx },
    "./LegalDocumentScreen": { LegalDocumentScreen: "LegalDocumentScreen" },
  });
  const navigation = {
    push: (route: string) => pushes.push(route),
    goBack: () => {
      backs++;
    },
  };
  for (const [name, own, target, destination] of [
    ["PrivacyPolicyScreen", "privacy", "terms", "TermsOfService"],
    ["TermsOfServiceScreen", "terms", "privacy", "PrivacyPolicy"],
  ] as const) {
    const Screen = h.load(
      fileURLToPath(new URL(`./${name}.tsx`, import.meta.url)),
    )[name] as (props: unknown) => Element;
    const screen = Screen({ navigation });
    screen.props.onOpenDocument(target);
    assert.equal(
      pushes.at(-1),
      destination,
      "push keeps the original route mounted, including its scroll position",
    );
    const count = pushes.length;
    screen.props.onOpenDocument(own);
    assert.equal(
      pushes.length,
      count,
      "do not push a duplicate of the current document",
    );
    screen.props.onBack();
  }
  assert.equal(backs, 2);
});

test("external policy and email links open safely without replacing the document", async (t) => {
  const f = documentFixture("privacy");
  t.after(f.h.unmount);
  await f.h.settle();
  const originalBody = f.markdown().props.children;
  for (const href of [
    "https://developers.google.com/terms/api-services-user-data-policy",
    "mailto:contact@chrono-ai.fun",
  ]) {
    assert.equal(f.markdown().props.onLinkPress(href), false);
  }
  await f.h.settle();
  assert.deepEqual(f.opened, [
    "https://developers.google.com/terms/api-services-user-data-policy",
    "mailto:contact@chrono-ai.fun",
  ]);
  assert.deepEqual(f.destinations, []);
  assert.equal(f.markdown().props.children, originalBody);
  assert.equal(f.alerts.length, 0);
});

for (const language of ["en", "zh-CN", "zh-TW"] as const) {
  test(`failed external links provide localized retry and preserve content (${language})`, async (t) => {
    const f = documentFixture("privacy", { language });
    t.after(f.h.unmount);
    await f.h.settle();
    const originalBody = f.markdown().props.children;
    f.setOpenURL(async () => {
      throw new Error("private native error");
    });
    const href = "https://other.example/policy";
    assert.equal(f.markdown().props.onLinkPress(href), false);
    await f.h.settle();
    const alert = f.alerts[0];
    assert.ok(alert);
    assert.equal(alert.title, translate(language, "Unable to open link"));
    assert.equal(
      alert.message,
      translate(language, "Try again, or open the link outside NyxID."),
    );
    assert.equal(alert.buttons[0]?.text, translate(language, "Cancel"));
    assert.equal(alert.buttons[0]?.style, "cancel");
    const retry = alert.buttons.find(
      (button) => button.text === translate(language, "Retry"),
    );
    assert.ok(retry?.onPress);
    f.setOpenURL(async () => {});
    retry.onPress();
    await f.h.settle();
    assert.deepEqual(f.opened, [href, href]);
    assert.equal(f.alerts.length, 1);
    assert.equal(f.markdown().props.children, originalBody);
    assert.equal(f.fetches(), 1);
  });
}

test("unsupported links never reach native Linking and fail visibly without unsafe retry", async (t) => {
  const f = documentFixture("terms");
  t.after(f.h.unmount);
  await f.h.settle();
  assert.equal(f.markdown().props.onLinkPress("javascript:alert(1)"), false);
  await f.h.settle();
  assert.equal(f.opened.length, 0);
  assert.equal(f.alerts.length, 1);
  assert.equal(f.alerts[0]?.buttons.length, 1);
  assert.equal(f.alerts[0]?.buttons[0]?.style, "cancel");
});

test("an external-open failure after leaving the document does not display a stale alert", async () => {
  const f = documentFixture("privacy");
  await f.h.settle();
  const pending = deferred<void>();
  f.setOpenURL(() => pending.promise);
  f.markdown().props.onLinkPress("https://other.example/policy");
  f.h.unmount();
  pending.reject(new Error("native open failed"));
  await f.h.settle();
  assert.equal(f.alerts.length, 0);
});

test("legal Back exposes a named button and invokes its callback", () => {
  let backs = 0;
  const harness = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": {
      Pressable: "Pressable",
      Text: "Text",
      StyleSheet: { create: <T>(styles: T) => styles },
    },
    "expo-blur": { BlurView: "BlurView" },
    "../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
  });
  const { BlurBackButton } = harness.load(
    fileURLToPath(
      new URL("../../components/BlurBackButton.tsx", import.meta.url),
    ),
  ) as { BlurBackButton: (props: unknown) => Element };
  harness.mount(() =>
    BlurBackButton({
      onPress: () => {
        backs++;
      },
    }),
  );
  assert.equal(harness.value().type, "Pressable");
  assert.equal(harness.value().props.accessibilityRole, "button");
  assert.equal(harness.value().props.accessibilityLabel, "Back");
  harness.value().props.onPress();
  assert.equal(backs, 1);
  harness.unmount();
});
