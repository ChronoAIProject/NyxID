import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import {
  CommonActions,
  StackActions,
  StackRouter,
} from "@react-navigation/routers";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import * as link from "./legalLink";

type Element = { type: unknown; props: Record<string, unknown> };
const nodes = (root: Element): Element[] => [
  root,
  ...[root.props.children]
    .flat(Infinity)
    .flatMap((child) =>
      child && typeof child === "object" && "props" in child
        ? nodes(child as Element)
        : [],
    ),
];
for (const inSheet of [false, true])
  for (const width of [393, 1194])
    for (const start of ["privacy", "terms"] as const) {
      test(`${start} cross-links pop to existing documents without duplicates in ${inSheet ? "sheet" : "root"} at ${width}pt`, async () => {
        const router = StackRouter({
          initialRouteName: inSheet ? "AccountSettings" : "Auth",
        });
        const options = {
          routeNames: [
            "Auth",
            "AccountSettings",
            "PrivacyPolicy",
            "TermsOfService",
          ],
          routeParamList: {},
          routeGetIdList: {},
        };
        let state = router.getInitialState(options);
        const apply = (
          action: Parameters<typeof router.getStateForAction>[1],
        ) => {
          const next = router.getStateForAction(state, action, options);
          assert.ok(next);
          state = router.getRehydratedState(next, options);
        };
        const navigation = {
          getState: () => state,
          push: (name: string) => apply(StackActions.push(name)),
          popTo: (name: string) => apply(StackActions.popTo(name)),
          goBack: () => apply(CommonActions.goBack()),
        };
        const names = {
          privacy: "PrivacyPolicy",
          terms: "TermsOfService",
        } as const;
        navigation.push(names[start]);
        const originalKey = state.routes.at(-1)!.key;
        const h = hookHarness<Element>(
          {
            "react/jsx-runtime": {
              jsx: (type: unknown, props: Element["props"]) => ({
                type,
                props,
              }),
              jsxs: (type: unknown, props: Element["props"]) => ({
                type,
                props,
              }),
            },
            "react-native": {
              View: "View",
              Text: "Text",
              ScrollView: "ScrollView",
              StyleSheet: { create: (s: unknown) => s },
              useWindowDimensions: () => ({ width, height: 834 }),
              Linking: {
                openURL: () =>
                  assert.fail("Internal cross-link reached native Linking"),
              },
              Alert: { alert: () => assert.fail("Cross-link failed") },
            },
            "react-native-markdown-display": "Markdown",
            "../../app/WorkspaceHeader": {
              useWorkspaceSheet: () => (inSheet ? {} : null),
            },
            "../../components/FloatingBackButton": {
              FloatingBackButton: "Back",
            },
            "../../components/ScreenContainer": {
              ScreenContainer: "ScreenContainer",
            },
            "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
            "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
            "../../theme/flowStyles": {
              createFlowStyles: () => ({ content: { flex: 1 } }),
            },
            "../../lib/telemetry": { capture() {} },
            "./legalDocumentUrl": {
              resolveLegalDocumentUrl: (key: string) =>
                `https://example.test/legal/${key}.md`,
            },
            "./legalLink": link,
            "./LegalDocumentScreen": { LegalDocumentScreen: "Document" },
          },
          {
            fetch: async (url: string) => ({
              ok: true,
              text: async () =>
                readFileSync(
                  new URL(
                    `../../../../frontend/public/legal/${url.includes("privacy") ? "privacy" : "terms"}.md`,
                    import.meta.url,
                  ),
                  "utf8",
                ),
            }),
          },
        );
        const privacy = h.load(
          fileURLToPath(new URL("./PrivacyPolicyScreen.tsx", import.meta.url)),
        ) as { PrivacyPolicyScreen: (props: unknown) => Element };
        const terms = h.load(
          fileURLToPath(new URL("./TermsOfServiceScreen.tsx", import.meta.url)),
        ) as { TermsOfServiceScreen: (props: unknown) => Element };
        const document = h.load(
          fileURLToPath(new URL("./LegalDocumentScreen.tsx", import.meta.url)),
        ) as { LegalDocumentScreen: (props: unknown) => Element };
        h.mount(() =>
          document.LegalDocumentScreen(
            (state.routes.at(-1)!.name === "PrivacyPolicy"
              ? privacy.PrivacyPolicyScreen({ navigation })
              : terms.TermsOfServiceScreen({ navigation })
            ).props,
          ),
        );
        for (let i = 0; i < 6; i++) {
          await h.settle();
          const current =
            state.routes.at(-1)!.name === "PrivacyPolicy" ? "privacy" : "terms";
          const target = current === "privacy" ? "terms" : "privacy";
          const markdown = nodes(h.value()).find((n) => n.type === "Markdown");
          assert.ok(markdown);
          assert.match(
            String(markdown.props.children),
            new RegExp(`\\]\\(/${target}\\)`),
          );
          assert.equal(
            (markdown.props.onLinkPress as (href: string) => boolean)(
              `/${target}`,
            ),
            false,
          );
          h.render();
          assert.equal(
            state.routes.filter((route) => route.name === names[start]).length,
            1,
          );
          assert.equal(
            state.routes.filter(
              (route) =>
                route.name === names[start === "privacy" ? "terms" : "privacy"],
            ).length,
            i % 2 === 0 ? 1 : 0,
          );
          assert.ok(state.routes.length <= 3);
        }
        assert.equal(state.routes.at(-1)!.key, originalKey);
        navigation.goBack();
        assert.equal(state.routes.length, 1);
        h.unmount();
      });
    }
