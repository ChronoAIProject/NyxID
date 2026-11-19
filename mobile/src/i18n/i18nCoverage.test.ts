import { exclusiveCharacters, wrongScriptCharacters } from './i18nScriptGuard';
import { translationConcatenations, missingRenderLanguageSubscription, moduleScopeTranslations } from "./i18nQualityGuard";
import { NON_TRANSLATABLE_TOKENS } from "./i18nLiteralGuard";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import test from "node:test";
import { untranslatedLiterals } from "./i18nLiteralGuard";
import cn from "./locales/zh-CN";
import tw from "./locales/zh-TW";
import { translate } from "./translate";
import {
  appLocale,
  setAppTranslationLanguage,
  translateApp,
} from "./appTranslation";
import { formatRelativeTimeFromMs } from "../lib/time";
import { formatDeviceLoginRelativeTime } from "../features/auth/deviceLoginPreview";
import { toolActivityTranslationKeys } from "../features/assistant/assistantActivityText";
import { capabilityLabels } from "../features/machines/agentMachineContracts";

const sources = (dir: URL): URL[] =>
  readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const file = new URL(entry.name + (entry.isDirectory() ? "/" : ""), dir);
    return entry.isDirectory()
      ? sources(file)
      : /\.tsx?$/.test(entry.name) &&
          !/(\.test\.|\.jest\.|Harness|fixture)/i.test(entry.name)
        ? [file]
        : [];
  });

test("both Chinese catalogs have identical keys, nonempty copy and intact interpolation", () => {
  assert.deepEqual(Object.keys(cn).sort(), Object.keys(tw).sort());
  for (const [language, catalog] of [
    ["zh-CN", cn],
    ["zh-TW", tw],
  ] as const)
    for (const [key, value] of Object.entries(catalog)) {
      assert.equal(key, key.trim(), `${language}: whitespace in ${key}`);
      assert.doesNotMatch(key, /^[,.;:·|]|[,;:·|]$/, `${language}: separator in ${key}`);
      assert.doesNotMatch(key, /\{\{(?:part|value)\d+\}\}/, `${language}: generic placeholder ${key}`);
      assert.doesNotMatch(key, /^(?:Increase|Decrease|Adjust|Toggle|Enable|Disable) \{\{(?:action|title|settingTitle|setting|label)\}\}/, `${language}: assembled setting action ${key}`);
      assert.ok(!NON_TRANSLATABLE_TOKENS.has(key), `${language}: non-translatable token ${key}`);
      assert.ok(value.trim(), `${language}: ${key}`);
      assert.deepEqual(
        (value.match(/\{\{\w+\}\}/g) ?? []).sort(),
        (key.match(/\{\{\w+\}\}/g) ?? []).sort(),
        `${language}: ${key}`,
      );
    }
});
test("authored Text, accessibility, placeholders and Alert copy cannot bypass translation", () => {
  const failures = ["features/", "components/", "app/"]
    .flatMap((area) => sources(new URL("../" + area, import.meta.url)))
    .flatMap((file) =>
      untranslatedLiterals(readFileSync(file, "utf8"), file.pathname).map(
        (literal) => `${file.pathname}: ${literal.phrase}`,
      ),
    );
  assert.deepEqual(failures, []);
});
test("literal guard detects new copy while ignoring data, formatter options and translated copy", () => {
  const code = `<><Text>Welcome</Text><Text>{user.name}</Text><Text>{t("Translated")}</Text><Text>{date.toLocaleString(appLocale(), { hour: "2-digit" })}</Text><TextInput placeholder="Email address" accessibilityLabel="Sign in" /></>; Alert.alert("Wrong password", t("Try again"), [{ text: "Cancel", style: "cancel" }]);`;
  assert.deepEqual(
    untranslatedLiterals(code).map((row) => row.phrase),
    ["Welcome", "Email address", "Sign in", "Wrong password", "Cancel"],
  );
  assert.deepEqual(
    untranslatedLiterals(
      "<><DynamicTypeText>Visible copy</DynamicTypeText><Animated.Text>Animated copy</Animated.Text></>",
    ).map((row) => row.phrase),
    ["Visible copy", "Animated copy"],
  );
});
test("selected app language updates helper copy, live label maps and relative dates without translating user data", () => {
  const now = Date.parse("2026-11-14T08:00:00Z");
  try {
    setAppTranslationLanguage("zh-CN");
    assert.equal(appLocale(), "zh-CN");
    assert.notEqual(translateApp("New chat"), "New chat");
    assert.notEqual(capabilityLabels.shell, "Shell commands");
    assert.match(formatRelativeTimeFromMs(now - 120000, now), /2.*分钟/);
    const name = "Alice’s research / 原样保留";
    assert.ok(translate("zh-CN", "Recent: {{name}}", { name }).includes(name));
    setAppTranslationLanguage("zh-TW");
    assert.match(formatRelativeTimeFromMs(now - 120000, now), /2.*分鐘/);
    setAppTranslationLanguage("en");
    assert.equal(capabilityLabels.shell, "Shell commands");
    assert.equal(formatRelativeTimeFromMs(now - 120000, now), "2m ago");
  } finally {
    setAppTranslationLanguage("en");
  }
});

test("relative times render in every app language without Intl.RelativeTimeFormat, as on Hermes", () => {
  const now = Date.parse("2026-11-14T08:00:00Z");
  const intl = Intl as { RelativeTimeFormat?: unknown };
  const original = intl.RelativeTimeFormat;
  delete intl.RelativeTimeFormat;
  try {
    for (const [language, minute, hour, day, device] of [
      ["en", "2m ago", "3h ago", "2d ago", "2 minutes ago"],
      ["zh-CN", "2 分钟前", "3 小时前", "2 天前", "2 分钟前"],
      ["zh-TW", "2 分鐘前", "3 小時前", "2 天前", "2 分鐘前"],
    ] as const) {
      setAppTranslationLanguage(language);
      assert.equal(formatRelativeTimeFromMs(now - 120000, now), minute);
      assert.equal(formatRelativeTimeFromMs(now - 3 * 3600000, now), hour);
      assert.equal(formatRelativeTimeFromMs(now - 2 * 86400000, now), day);
      assert.equal(
        formatDeviceLoginRelativeTime(new Date(now - 120000).toISOString(), now),
        device,
      );
    }
  } finally {
    intl.RelativeTimeFormat = original;
    setAppTranslationLanguage("en");
  }
  // Hermes lacks these constructors; DisplayNames stays behind a try/catch fallback.
  const unsupported =
    /new Intl\.(RelativeTimeFormat|PluralRules|ListFormat|Segmenter)\b/;
  assert.deepEqual(
    ["features/", "components/", "app/", "lib/", "i18n/"]
      .flatMap((area) => sources(new URL("../" + area, import.meta.url)))
      .filter((url) => unsupported.test(readFileSync(url, "utf8")))
      .map((url) => url.pathname),
    [],
  );
});

test("every statically authored translation key has both Chinese translations", async () => {
  const ts = await import("typescript");
  const files = ["features/", "components/", "app/", "lib/api/"]
    .flatMap((area) => sources(new URL("../" + area, import.meta.url)))
    .map((url) =>
      ts.createSourceFile(
        url.pathname,
        readFileSync(url, "utf8"),
        ts.ScriptTarget.Latest,
        true,
      ),
    );
  const constants = new Map<string, string>();
  for (const file of files) {
    const collect = (node: import("typescript").Node) => {
      if (
        ts.isVariableDeclaration(node) &&
        ts.isIdentifier(node.name) &&
        /^[A-Z][A-Z_]+$/.test(node.name.text) &&
        node.initializer &&
        ts.isStringLiteralLike(node.initializer)
      )
        constants.set(node.name.text, node.initializer.text);
      ts.forEachChild(node, collect);
    };
    collect(file);
  }
  const missing = new Set<string>();
  for (const file of files) {
    const key = (node: import("typescript").Node) => {
      if (ts.isStringLiteralLike(node)) {
        if (!(node.text in cn) || !(node.text in tw)) missing.add(node.text);
      } else if (ts.isConditionalExpression(node)) {
        key(node.whenTrue);
        key(node.whenFalse);
      } else if (ts.isIdentifier(node) && constants.has(node.text)) {
        const phrase = constants.get(node.text)!;
        if (!(phrase in cn) || !(phrase in tw)) missing.add(phrase);
      }
    };
    const visit = (node: import("typescript").Node) => {
      if (
        ts.isCallExpression(node) &&
        /^(?:t|translateApp)$|\.t$|translate\.current$/.test(
          node.expression.getText(file),
        ) &&
        node.arguments[0]
      )
        key(node.arguments[0]);
      if (ts.isJsxAttribute(node) && node.name.getText(file) === "message" &&
          (ts.isJsxSelfClosingElement(node.parent.parent) || ts.isJsxOpeningElement(node.parent.parent)) &&
          node.parent.parent.tagName.getText(file) === "Translation" && node.initializer)
        key(node.initializer);
      ts.forEachChild(node, visit);
    };
    visit(file);
  }
  assert.deepEqual([...missing].sort(), []);
});

test("translation results cannot be concatenated into sentences", () => {
  const failures = ["features/", "components/", "app/"]
    .flatMap((area) => sources(new URL("../" + area, import.meta.url)))
    .flatMap((file) => translationConcatenations(readFileSync(file, "utf8"), file.pathname)
      .map((line) => `${file.pathname}:${line}`));
  assert.deepEqual(failures, []);
  assert.equal(translationConcatenations('t("Hello") + t("world")').length, 1);
  assert.equal(translationConcatenations('`Recent: ${t("Running")}`').length, 1);
  assert.equal(translationConcatenations('label += translateApp("suffix")').length, 1);
  assert.deepEqual(translationConcatenations('t("Recent: {{name}}", { name })'), []);
});

test("rendered bridge translations subscribe to the app language", () => {
  const failures = ["features/", "components/", "app/"]
    .flatMap((area) => sources(new URL("../" + area, import.meta.url)))
    .filter((file) => file.pathname.endsWith(".tsx"))
    .flatMap((file) => missingRenderLanguageSubscription(readFileSync(file, "utf8"), file.pathname)
      .map((line) => `${file.pathname}:${line}`));
  assert.deepEqual(failures, []);
  assert.equal(missingRenderLanguageSubscription('function Page() { return <Text>{translateApp("Hello")}</Text> }').length, 1);
  assert.equal(missingRenderLanguageSubscription('function Page() { const title = translateApp("Hello"); return <Text>{title}</Text> }').length, 1);
  assert.equal(missingRenderLanguageSubscription('const Page = memo(() => { const title = translateApp("Hello"); return <Text>{title}</Text> })').length, 1);
  assert.deepEqual(missingRenderLanguageSubscription('function Page() { useAppLanguage(); return <Text>{translateApp("Hello")}</Text> }'), []);
  assert.deepEqual(missingRenderLanguageSubscription('function Page() { return <Button onPress={() => Alert.alert(translateApp("Hello"))} /> }'), []);
});

test("N13 production translations never freeze copy at module scope", () => {
  const failures = sources(new URL("../", import.meta.url)).flatMap((file) =>
    moduleScopeTranslations(readFileSync(file, "utf8"), file.pathname)
      .map((line) => `${file.pathname}:${line}`));
  assert.deepEqual(failures, []);
});

test("N13 module-scope AST guard distinguishes cached copy from deferred formatters", () => {
  assert.deepEqual(moduleScopeTranslations(`
    import { translateApp as label } from "./appTranslation";
    const days = [label("Sundays"), translateApp("Mondays")];
    const map = { day: translate("zh-CN", "Tuesdays") };
    class Labels { static day = translateApp("Wednesdays"); }
  `), [3, 3, 4, 5]);
  assert.deepEqual(moduleScopeTranslations(`
    // translateApp("not executed")
    const source = 'translateApp("not executed")';
    const getter = { get label() { return translateApp("Sundays"); } };
    const format = () => translateApp("Mondays");
    function render() { return translate("zh-CN", "Tuesdays"); }
  `), []);
});

test("Chinese catalogs use the correct script including OpenCC mappings with multiple targets", () => {
  const simplified = exclusiveCharacters("STCharacters.txt");
  const traditional = exclusiveCharacters("TSCharacters.txt");
  // Multiple traditional choices do not make these valid Traditional characters.
  for (const character of "开证发复获") assert.ok(simplified.has(character));
  for (const character of "開證發髮複獲") assert.ok(traditional.has(character));
  for (const [language, catalog] of [["zh-CN", cn], ["zh-TW", tw]] as const)
    for (const [key, value] of Object.entries(catalog))
      assert.deepEqual(wrongScriptCharacters(language, value), [], `${language}: ${key}`);
  for (const shared of "刷窗缺着著") {
    assert.ok(!simplified.has(shared));
    assert.ok(!traditional.has(shared));
  }
});

test("tool activities translate one complete template for every operation state", () => {
  for (const key of toolActivityTranslationKeys()) {
    assert.ok(key in cn, `zh-CN: ${key}`);
    assert.ok(key in tw, `zh-TW: ${key}`);
  }
});
