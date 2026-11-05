import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import MarkdownIt from "markdown-it";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import {
  darkColors,
  darkSheetColors,
  lightColors,
  lightSheetColors,
} from "../../theme/mobileTheme";

function contrast(foreground: string, background: string) {
  const luminance = (hex: string) =>
    [1, 3, 5]
      .map((offset) => parseInt(hex.slice(offset, offset + 2), 16) / 255)
      .map((channel) =>
        channel <= 0.04045
          ? channel / 12.92
          : ((channel + 0.055) / 1.055) ** 2.4,
      )
      .reduce(
        (sum, channel, index) =>
          sum + channel * [0.2126, 0.7152, 0.0722][index]!,
        0,
      );
  const [low, high] = [luminance(foreground), luminance(background)].sort(
    (a, b) => a - b,
  );
  return (high! + 0.05) / (low! + 0.05);
}

test("canonical legal notices keep AA contrast through Light, Dark and System changes in page and sheet themes", async () => {
  let scheme = "dark";
  let theme: any;
  let sheet = false;
  const jsx = (type: unknown, props: unknown) => ({ type, props });
  const providerHarness = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { useColorScheme: () => scheme },
    "expo-secure-store": {
      getItemAsync: async () => null,
      setItemAsync: async () => {},
    },
  });
  const { ThemeProvider } = providerHarness.load(
    fileURLToPath(new URL("../../theme/ThemeContext.tsx", import.meta.url)),
  ) as { ThemeProvider: (props: unknown) => any };
  providerHarness.mount(() => {
    const provider = ThemeProvider({ children: null });
    if (provider) theme = provider.props.value;
    return provider;
  });
  await providerHarness.settle();
  const harness = hookHarness<any>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-native": { StyleSheet: { create: (value: unknown) => value } },
    "../../app/WorkspaceHeader": {},
    "../../components/BlurBackButton": {},
    "../../components/ScreenContainer": {},
    "../../components/Skeleton": {},
    "../../lib/env": { NYXID_LEGAL_BASE_URL: "https://example.test" },
    "../../lib/telemetry": {},
    "../../theme/flowStyles": {},
    "../../theme/ThemeContext": {
      useTheme: () => ({
        ...theme,
        colors: sheet
          ? theme.mode === "dark"
            ? darkSheetColors
            : lightSheetColors
          : theme.colors,
      }),
    },
    "react-native-markdown-display": { __esModule: true, default: "Markdown" },
  });
  const { LegalDocumentMarkdown } = harness.load(
    fileURLToPath(new URL("./LegalDocumentScreen.tsx", import.meta.url)),
  ) as { LegalDocumentMarkdown: (props: { content: string }) => any };
  const terms = readFileSync(
    new URL("../../../../frontend/public/legal/terms.md", import.meta.url),
    "utf8",
  );
  assert.ok(
    new MarkdownIt()
      .parse(terms, {})
      .some((token) => token.type === "blockquote_open"),
    "the canonical IMPORTANT NOTICE uses the blockquote rendering rule",
  );
  harness.mount(() => LegalDocumentMarkdown({ content: terms }));
  for (const preference of ["light", "dark", "system"]) {
    theme.setPreference(preference);
    for (scheme of ["light", "dark"]) {
      providerHarness.render();
      const mode = preference === "system" ? scheme : preference;
      assert.equal(theme.mode, mode);
      for (sheet of [false, true]) {
        const colors = sheet
          ? mode === "dark"
            ? darkSheetColors
            : lightSheetColors
          : mode === "dark"
            ? darkColors
            : lightColors;
        const rendered = harness.render();
        assert.equal(rendered.type, "Markdown");
        assert.equal(
          rendered.props.children,
          terms,
          "canonical wording is passed through unchanged",
        );
        const styles = rendered.props.style;
        assert.equal(styles.blockquote?.backgroundColor, colors.cardSoft);
        assert.equal(styles.blockquote.borderColor, colors.border);
        for (const role of [
          "body",
          "paragraph",
          "strong",
          "heading1",
          "heading2",
          "heading3",
          "link",
        ]) {
          const ratio = contrast(
            styles[role].color,
            styles.blockquote.backgroundColor,
          );
          assert.ok(ratio >= 4.5, `${role}: ${ratio.toFixed(2)}:1`);
        }
        assert.ok(styles.blockquote.paddingHorizontal > 0);
        assert.equal(
          styles.blockquote.height,
          undefined,
          "notice can grow with native text scaling",
        );
      }
    }
  }
  harness.unmount();
  providerHarness.unmount();
});
