import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { translate } from "../../i18n/translate";

type Element = { type: string; props: Record<string, unknown> };
test("compact Pager keeps three columns, 44pt actions, page announcement and disabled boundary/loading/offline state", () => {
  for (const [page, disabled] of [
    [1, false],
    [3, false],
    [8, false],
    [3, true],
  ] as const) {
    const changes: number[] = [];
    const harness = hookHarness<Element>({
      "react/jsx-runtime": {
        jsx: (type: string, props: Element["props"]) => ({ type, props }),
        jsxs: (type: string, props: Element["props"]) => ({ type, props }),
      },
      "react-native": {
        Pressable: "Pressable",
        Text: "Text",
        View: "View",
        StyleSheet: { create: (s: unknown) => s },
      },
      "lucide-react-native": {
        ChevronLeft: "ChevronLeft",
        ChevronRight: "ChevronRight",
      },
      "../../theme/ThemeContext": {
        useTheme: () => ({
          colors: {
            textMuted: "muted",
            primary: "primary",
            navActive: "pressed",
          },
        }),
      },
    });
    const { Pager } = harness.load(
      fileURLToPath(new URL("./Pager.tsx", import.meta.url)),
    ) as { Pager: (props: unknown) => Element };
    const root = harness.mount(() =>
      Pager({
        page,
        total: 8,
        disabled,
        onChange: (n: number) => changes.push(n),
      }),
    );
    const rowStyle = root.props.style as {
      flexDirection: string;
      flexWrap: string;
    };
    assert.equal(rowStyle.flexDirection, "row");
    assert.equal(rowStyle.flexWrap, "wrap");
    const [previous, label, next] = root.props.children as Element[];
    assert.equal(label!.props.accessibilityLabel, `Page ${page} of 8`);
    assert.equal(label!.props.children, `${page} / 8`);
    for (const [action, blocked, destination, glyph, hint] of [
      [
        previous!,
        disabled || page === 1,
        page - 1,
        "ChevronLeft",
        "Shows the previous page of results",
      ],
      [
        next!,
        disabled || page === 8,
        page + 1,
        "ChevronRight",
        "Shows the next page of results",
      ],
    ] as const) {
      assert.equal(action.props.disabled, blocked);
      assert.equal(
        (action.props.accessibilityState as { disabled: boolean }).disabled,
        blocked,
      );
      const style = (
        action.props.style as (s: {
          pressed: boolean;
        }) => Array<
          | false
          | {
              minWidth?: number;
              minHeight?: number;
              opacity?: number;
              backgroundColor?: string;
            }
        >
      )({ pressed: false });
      assert.ok(style[0] && style[0].minWidth! >= 44);
      assert.ok(style[0] && style[0].minHeight! >= 44);
      const children = action.props.children as Element[];
      const label = children.find((node) => node.type === "Text")!;
      const icon = children.find((node) => node.type === glyph)!;
      assert.equal(icon.props.size, 16);
      assert.equal(icon.props.accessible, false);
      assert.equal(icon.props.color, blocked ? "muted" : "primary");
      assert.equal(action.props.accessibilityHint, hint);
      assert.equal(
        (label.props.style as Array<{ color?: string }>)[1]!.color,
        blocked ? "muted" : "primary",
      );
      assert.equal(
        (label.props.style as Array<{ flexShrink?: number }>)[0]!.flexShrink,
        1,
      );
      for (const pressed of [false, true]) {
        const pressedStyle = (
          action.props.style as (s: {
            pressed: boolean;
          }) => Array<false | { opacity?: number }>
        )({ pressed });
        assert.ok(
          pressedStyle.every((item) => !item || item.opacity === undefined),
        );
        assert.ok(
          (label.props.style as Array<{ opacity?: number }>).every(
            (item) => item.opacity === undefined,
          ),
        );
      }
      (action.props.onPress as () => void)();
      if (!blocked) assert.equal(changes.at(-1), destination);
    }
    assert.equal(changes.length, disabled ? 0 : page === 3 ? 2 : 1);
    harness.unmount();
  }
});

test("Pager direction labels and accessibility hints use the selected app language", () => {
  for (const language of ["zh-CN", "zh-TW"] as const) {
    const harness = hookHarness<Element>({
      "react-native": {
        Pressable: "Pressable",
        Text: "Text",
        View: "View",
        StyleSheet: { create: (value: unknown) => value },
      },
      "lucide-react-native": {
        ChevronLeft: "ChevronLeft",
        ChevronRight: "ChevronRight",
      },
      "../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
      "../../i18n/LanguageProvider": {
        useAppLanguage: () => ({
          t: (phrase: string, values?: Record<string, string | number>) =>
            translate(language, phrase, values),
        }),
      },
    });
    const { Pager } = harness.load(
      fileURLToPath(new URL("./Pager.tsx", import.meta.url)),
    ) as { Pager: (props: unknown) => Element };
    const root = harness.mount(() =>
      Pager({ page: 3, total: 8, onChange: () => {} }),
    );
    const [previous, page, next] = root.props.children as Element[];
    assert.equal(
      previous!.props.accessibilityLabel,
      translate(language, "Previous"),
    );
    assert.equal(next!.props.accessibilityLabel, translate(language, "Next"));
    for (const action of [previous!, next!])
      assert.match(String(action.props.accessibilityHint), /[\u3400-\u9fff]/);
    assert.equal(
      page!.props.accessibilityLabel,
      translate(language, "Page {{page}} of {{total}}", { page: 3, total: 8 }),
    );
    assert.equal(page!.props.children, "3／8");
    harness.unmount();
  }
});
