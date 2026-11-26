import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { hookHarness } from "../assistantHookTestHarness";
import { matchesLanguage } from "./dictationLanguageLabels";

test("picker item keys remain unique across automatic, Suggested and All sections", () => {
  type Element = { type: unknown; props: Record<string, any> };
  const h = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      Modal: "Modal",
      SectionList: "SectionList",
      StyleSheet: { create: (value: unknown) => value },
    },
    "react-native-safe-area-context": {
      SafeAreaProvider: "Provider",
      SafeAreaView: "SafeArea",
    },
    "../../../i18n/LanguageProvider": {
      useAppLanguage: () => ({ t: (value: string) => value }),
    },
    "../../../theme/ThemeContext": { useTheme: () => ({ colors: {} }) },
    "../../../components/settings/SettingsList": {},
    "../../../components/DynamicTypeText": { DynamicTypeText: "Text" },
    "lucide-react-native": {},
    "./dictationLanguageLabels": { matchesLanguage },
  });
  const { DictationLanguagePicker } = h.load(
    fileURLToPath(new URL("./DictationLanguagePicker.tsx", import.meta.url)),
  ) as typeof import("./DictationLanguagePicker");
  const language: Parameters<typeof DictationLanguagePicker>[0]["language"] = {
    loading: false,
    listUnavailable: false,
    explanation: undefined,
    value: "automatic",
    resolved: "ja-JP",
    label: "Japanese",
    summary: "Japanese",
    suggested: ["ja-JP"],
    manualEntry: false,
    retry() {},
    openSpeechSettings() {},
    options: [
      { value: "automatic", label: "Device language", detail: "Japanese" },
      {
        value: "ja-JP",
        label: "Japanese",
        detail: "日本語 · ja-JP",
        english: "Japanese",
      },
    ],
    choose() {},
  };
  h.mount(() => DictationLanguagePicker({ language, onClose() {} }));
  const find = (node: Element): Element | undefined =>
    node.type === "SectionList"
      ? node
      : [node.props.children]
          .flat(Infinity)
          .filter((value) => value?.props)
          .map(find)
          .find(Boolean);
  const list = find(h.value())!;
  const keys = list.props.sections.flatMap((section: { data: unknown[] }) =>
    section.data.map(list.props.keyExtractor),
  );
  assert.equal(keys.length, 3);
  assert.equal(new Set(keys).size, keys.length);
  h.unmount();
});
