import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { translate } from "../../i18n/translate";
import type { Language } from "../../i18n/language";

type Element = { type: unknown; props: Record<string, any> };
function elements(node: Element): Element[] {
  return [
    node,
    ...[node.props.children]
      .flat(Infinity)
      .flatMap((child) => (child?.props ? elements(child) : [])),
  ];
}

test("Language uses native labels, changes interface immediately and offers persistence retry", () => {
  let language: Language = "en";
  let error = false;
  let retries = 0;
  const selected: string[] = [];
  const jsx = (type: unknown, props: Element["props"]) => ({ type, props });
  const h = hookHarness<Element>({
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "lucide-react-native": { Languages: "Languages" },
    "../../components/settings/SettingsList": Object.fromEntries(
      ["SettingsPage", "SettingsGroup", "SettingsMenuRow", "SettingsRow"].map(
        (name) => [name, name],
      ),
    ),
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        language,
        persistenceError: error,
        choose: (next: Language) => {
          language = next;
          selected.push(next);
        },
        retryPersistence: () => {
          retries++;
        },
        t: (phrase: string) => translate(language, phrase),
      }),
    },
  });
  const { LanguageSettingsScreen } = h.load(
    fileURLToPath(new URL("./LanguageSettingsScreen.tsx", import.meta.url)),
  ) as {
    LanguageSettingsScreen(props: unknown): Element;
  };
  h.mount(() => LanguageSettingsScreen({}));
  const menu = () =>
    elements(h.value()).find((n) => n.type === "SettingsMenuRow")!;
  assert.deepEqual(JSON.parse(JSON.stringify(menu().props.options)), [
    { value: "en", label: "English" },
    { value: "zh-CN", label: "简体中文" },
    { value: "zh-TW", label: "繁體中文" },
  ]);
  menu().props.onChange("zh-CN");
  h.render();
  assert.equal(menu().props.label, "语言");
  assert.equal(menu().props.value, "zh-CN");
  menu().props.onChange("zh-TW");
  h.render();
  assert.equal(menu().props.label, "語言");
  menu().props.onChange("unsupported");
  assert.deepEqual(selected, ["zh-CN", "zh-TW"]);
  error = true;
  h.render();
  const retry = elements(h.value()).find((n) => n.type === "SettingsRow")!;
  assert.equal(retry.props.label, "重試儲存語言");
  retry.props.onPress();
  assert.equal(retries, 1);
  h.unmount();
});
