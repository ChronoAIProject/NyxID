import { translate, type TranslationValues } from "../../i18n/translate";
import type { Language } from "../../i18n/language";
import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import { renderSegmentControl } from "../../components/SegmentControl.testHarness";
import type { AddedSkillItem } from "./SkillLibrary";
import type { CatalogSkill } from "./pluginSkillsTypes";
import * as designTokens from "../../theme/designTokens";

type Element = { type: unknown; props: Record<string, any> };

function render(root: Element): Element {
  if (typeof root.type === "function") return render(root.type(root.props));
  return {
    ...root,
    props: {
      ...root.props,
      children: [root.props.children]
        .flat(Infinity)
        .map((child) =>
          child && typeof child === "object" && "props" in child
            ? render(child)
            : child,
        ),
    },
  };
}

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

// Settings rows and groups carry their visible text in props, not children.
const TEXT_PROPS = ["title", "label", "detail", "value", "footer"];
function textContent(root: Element): string {
  return [
    ...TEXT_PROPS.map((name) =>
      typeof root.props[name] === "string" ? root.props[name] : "",
    ),
    root.props.children,
  ]
    .flat(Infinity)
    .map((child) =>
      child && typeof child === "object" && "props" in child
        ? textContent(child)
        : typeof child === "string" || typeof child === "number"
          ? String(child)
          : "",
    )
    .join("");
}

const pins: AddedSkillItem[] = [
  {
    id: "opaque-skill-1",
    name: "cloudflare-dns-helper",
    description: "Safely review and update DNS records.",
    version: "2026.10-exact",
    disabled: false,
  },
  {
    id: "opaque-skill-2",
    name: "weekly_report",
    description: "Summarize sales figures every week.",
    version: "1.0.3",
    disabled: true,
  },
];
const catalog: CatalogSkill[] = [
  {
    id: "opaque-skill-2",
    name: "weekly_report",
    description: "Summarize sales figures every week.",
  },
  {
    id: "opaque-skill-3",
    name: "Document Expert",
    description: "Read and prepare business documents.",
  },
];

function setup(
  overrides: Record<string, unknown> = {},
  language: Language = "en",
) {
  const calls: { kind: string; args: unknown[] }[] = [];
  const record =
    (kind: string) =>
    (...args: unknown[]) =>
      calls.push({ kind, args });
  const harness = hookHarness<Element>({
    "../../i18n/LanguageProvider": {
      useAppLanguage: () => ({
        t: (phrase: string, values?: TranslationValues) =>
          translate(language, phrase, values),
      }),
    },
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
      Fragment: "Fragment",
    },
    "react-native": {
      ...Object.fromEntries(
        ["ActivityIndicator", "Pressable", "Text", "TextInput", "View"].map(
          (name) => [name, name],
        ),
      ),
      Keyboard: { dismiss: record("keyboard") },
      StyleSheet: { create: (styles: unknown) => styles },
    },
    "lucide-react-native": Object.fromEntries(
      ["BookOpen", "Search", "X"].map((name) => [name, name]),
    ),
    "../../components/settings/SettingsList": {
      SettingsGroup: "SettingsGroup",
      SettingsRow: "SettingsRow",
    },
    "../../components/Skeleton": { SkeletonRows: "SkeletonRows" },
    "../../components/workspace/WorkspaceUI": { Notice: "Notice" },
    "../../components/SegmentControl": {
      SegmentControl: renderSegmentControl,
    },
    "../../theme/designTokens": designTokens,
    "../../theme/ThemeContext": {
      useTheme: () => ({ colors: { primaryTone: {} } }),
    },
  });
  const source = harness.load(
    fileURLToPath(new URL("./SkillLibrary.tsx", import.meta.url)),
  ) as {
    SkillLibrary: (props: Record<string, unknown>) => Element;
    skillDisplayName: (name: string) => string;
  };
  const addedState = {
    items: pins,
    loading: false,
    known: true,
    onRetry: record("retry-added"),
  };
  const catalogState = {
    items: catalog,
    loading: false,
    known: true,
    onRetry: record("retry-catalog"),
  };
  const tree = render(
    harness.mount(() =>
      source.SkillLibrary({
        agentName: "Research agent",
        query: "",
        added: addedState,
        catalog: catalogState,
        page: 1,
        totalPages: 1,
        onQueryChange: record("query"),
        onPageChange: record("page"),
        onOpenSkill: record("open"),
        ...overrides,
      }),
    ),
  );
  const find = (predicate: (item: Element) => boolean) => {
    const item = elements(tree).find(predicate);
    assert.ok(item, "Expected a matching library control");
    return item;
  };
  return {
    tree,
    calls,
    displayName: source.skillDisplayName,
    text: () => textContent(tree),
    rows: () =>
      elements(tree).filter(
        (item) => item.props.accessibilityHint === "View skill details",
      ),
    // Added pins speak their exact version; catalog rows never do.
    addedRows: () =>
      elements(tree).filter(
        (item) =>
          item.props.accessibilityHint === "View skill details" &&
          String(item.props.accessibilityLabel).includes(", version "),
      ),
    catalogRows: () =>
      elements(tree).filter(
        (item) =>
          item.props.accessibilityHint === "View skill details" &&
          !String(item.props.accessibilityLabel).includes(", version "),
      ),
    control: (label: string) =>
      find(
        (item) =>
          item.props.accessibilityLabel === label || item.props.label === label,
      ),
    hasText: (label: string) => textContent(tree).includes(label),
  };
}

test("Added gives readable names, descriptions and exact package versions with separate enabled state", () => {
  const env = setup();
  assert.match(env.text(), /Added to this agent · 2 of 16/);
  assert.match(env.text(), /Cloudflare dns helper/);
  assert.match(env.text(), /Safely review and update DNS records/);
  assert.match(env.text(), /Weekly report/);
  assert.equal(env.addedRows().length, 2);
  assert.deepEqual(
    env.addedRows().map((row) => row.props.value),
    ["2026.10-exact", "1.0.3 · Disabled"],
  );
  // Added and Discover share one page; there is no nested selector.
  assert.throws(() => env.control("Added skills"));
  assert.throws(() => env.control("Discover skills"));
  assert.match(env.text(), /Added to this agent[\s\S]*Discover/);
});

test("Added searches names, descriptions and slug words using spaces, case and repeated whitespace", () => {
  for (const query of [
    "CloudFlare DNS",
    "cloudflare-dns",
    "  CLOUDFLARE   DNS  ",
    "review and update dns",
  ]) {
    const env = setup({ query });
    assert.equal(env.addedRows().length, 1, query);
    assert.match(
      env.addedRows()[0]!.props.accessibilityLabel,
      /^cloudflare-dns-helper,/,
    );
    assert.match(env.text(), /2 of 16/);
  }
  assert.match(
    setup({ query: "weekly report" }).addedRows()[0]!.props.accessibilityLabel,
    /^weekly_report,/,
  );
  assert.match(
    setup({ query: "sales figures" }).addedRows()[0]!.props.accessibilityLabel,
    /^weekly_report,/,
  );
});

test("one search field filters Added and searches the catalog without attaching or changing any skill", () => {
  const env = setup({ query: "cloud" });
  const input = env.control("Search skills");
  assert.equal(input.props.value, "cloud");
  assert.equal(input.props.maxLength, 200);
  input.props.onChangeText("another");
  env.control("Clear skill search").props.onPress();
  assert.deepEqual(env.calls, [
    { kind: "query", args: ["another"] },
    { kind: "query", args: [""] },
  ]);
  assert.throws(() => env.control("Search added skills"));
  assert.throws(() => env.control("Search skill catalog"));
});

test("catalog opens read-only review with the original opaque id and exact name, never the display label", () => {
  const env = setup({ query: "keyword" });
  assert.equal(
    env.catalogRows().length,
    2,
    "catalog results are server-filtered, not filtered a second time by Added search",
  );
  assert.match(env.text(), /Find a skill for Research agent/);
  assert.match(
    env.catalogRows()[0]!.props.accessibilityLabel,
    /^weekly_report, Added$/,
  );
  assert.match(
    env.catalogRows()[1]!.props.accessibilityLabel,
    /^Document Expert, View versions$/,
  );
  assert.match(textContent(env.catalogRows()[0]!), /Weekly report/);
  env.catalogRows()[0]!.props.onPress();
  env.catalogRows()[1]!.props.onPress();
  assert.deepEqual(env.calls, [
    {
      kind: "open",
      args: [
        "opaque-skill-2",
        "weekly_report",
        "Summarize sales figures every week.",
      ],
    },
    {
      kind: "open",
      args: [
        "opaque-skill-3",
        "Document Expert",
        "Read and prepare business documents.",
      ],
    },
  ]);
  assert.throws(() => env.control("Attach exact version"));
  assert.throws(() => env.control("Detach skill"));
});

test("Added rows preserve original identity and exact version in accessibility while labels stay readable", () => {
  const env = setup();
  assert.equal(
    env.rows()[0]!.props.accessibilityLabel,
    "cloudflare-dns-helper, version 2026.10-exact, Enabled",
  );
  env.rows()[0]!.props.onPress();
  assert.deepEqual(env.calls, [
    {
      kind: "open",
      args: [
        "opaque-skill-1",
        "cloudflare-dns-helper",
        "Safely review and update DNS records.",
      ],
    },
  ]);
  assert.equal(env.displayName("NyxID OAuth Expert"), "NyxID OAuth Expert");
  assert.equal(env.displayName("already-readable"), "Already readable");
  assert.equal(env.displayName("camelCase-helper"), "camelCase-helper");
});

test("initial loading and unknown state never claim no skills or show a false known count", () => {
  const state = { items: [], known: false, loading: true, onRetry: () => {} };
  const added = setup({ added: state });
  assert.equal(added.control("Loading added skills").type, "SkeletonRows");
  assert.equal(added.hasText("No skills added yet"), false);
  assert.equal(added.hasText("Added · 0"), false);
  assert.equal(added.hasText("0 of 16"), false);
  const discover = setup({ catalog: state });
  assert.equal(
    discover.control("Searching skill catalog").type,
    "SkeletonRows",
  );
  assert.equal(discover.hasText("No skills found"), false);
});

test("failed queries keep their own retry and never masquerade as successful empty results", () => {
  const retries: string[] = [];
  const added = setup({
    added: {
      items: [],
      known: true,
      loading: false,
      error: "Added query failed",
      onRetry: () => retries.push("added"),
    },
  });
  const error = elements(added.tree).find((item) => item.type === "Notice");
  assert.equal(error?.props.message, "Added query failed");
  assert.equal(added.hasText("No skills added yet"), false);
  added.control("Reload skills").props.onPress();
  const discover = setup({
    catalog: {
      items: [],
      known: true,
      loading: false,
      error: "Catalog query failed",
      onRetry: () => retries.push("catalog"),
    },
  });
  assert.equal(discover.hasText("No skills found"), false);
  discover.control("Retry skill catalog").props.onPress();
  assert.deepEqual(retries, ["added", "catalog"]);
});

test("empty Added and local search no-match explain the next action independently from empty catalog", () => {
  const added = setup({
    added: { items: [], known: true, loading: false, onRetry: () => {} },
  });
  assert.match(added.text(), /No skills added yet/);
  assert.match(added.text(), /Skills do not grant service access/);
  assert.match(added.text(), /Add a skill from Discover below/);
  assert.throws(() => added.control("Discover skills"));
  const noMatch = setup({ query: "no such workflow" });
  assert.match(noMatch.text(), /No matching added skills/);
  assert.equal(noMatch.hasText("No skills added yet"), false);
  const catalogEmpty = setup({
    catalog: { items: [], known: true, loading: false, onRetry: () => {} },
  });
  assert.match(catalogEmpty.text(), /No skills found/);
  assert.match(
    catalogEmpty.text(),
    /Only skills available through your Ornn access appear here/,
  );
});

test("catalog pagination has explicit boundaries, busy state and no Added pagination", () => {
  const env = setup({ page: 2, totalPages: 4 });
  assert.match(env.text(), /2 \/ 4/);
  env.control("Previous").props.onPress();
  env.control("Next").props.onPress();
  assert.deepEqual(env.calls, [
    { kind: "page", args: [1] },
    { kind: "page", args: [3] },
  ]);
  assert.equal(
    setup({ page: 1, totalPages: 4 }).control("Previous").props.disabled,
    true,
  );
  assert.equal(
    setup({ page: 4, totalPages: 4 }).control("Next").props.disabled,
    true,
  );
  const busy = setup({
    page: 2,
    totalPages: 4,
    catalog: { items: catalog, known: true, loading: true, onRetry: () => {} },
  });
  assert.equal(busy.control("Previous").props.disabled, true);
  assert.equal(busy.control("Next").props.disabled, true);
  assert.equal(
    busy.catalogRows().length,
    2,
    "cached results remain readable during refresh",
  );
  assert.equal(busy.hasText("Searching skill catalog"), false);
  assert.throws(() => busy.control("Searching skill catalog"));
  // Paging belongs to the catalog section only; Added is never paged.
  assert.match(setup({ totalPages: 4 }).text(), /Page 1 \/ 4/);
});

test("blank descriptions offer review guidance rather than another opaque slug", () => {
  const env = setup({
    catalog: {
      items: [{ id: "blank", name: "empty-description", description: "   " }],
      known: true,
      loading: false,
      onRetry: () => {},
    },
  });
  assert.match(
    env.text(),
    /Open this skill to review its details and available versions/,
  );
});

test("offline unknown sections explain missing cache without claiming empty skills or showing busy states", () => {
  const unknown = {
    items: [],
    known: false,
    loading: false,
    onRetry: () => {},
  };
  const env = setup({ offline: true, added: unknown, catalog: unknown });
  assert.match(env.text(), /Added skills unavailable offline/);
  assert.match(env.text(), /Skill catalog unavailable offline/);
  assert.match(env.text(), /Reconnect to load this agent's skills/);
  assert.equal(env.hasText("No skills added yet"), false);
  assert.equal(env.hasText("No skills found"), false);
  assert.equal(env.hasText("Loading added skills"), false);
  assert.equal(env.hasText("Searching skill catalog"), false);
});

test("offline cached rows remain reviewable and searchable while remote paging and retries are disabled", () => {
  const env = setup({
    offline: true,
    page: 2,
    totalPages: 4,
    added: {
      items: pins,
      known: true,
      loading: false,
      error: "Old error",
      onRetry: () => {},
    },
    catalog: {
      items: catalog,
      known: true,
      loading: false,
      error: "Old error",
      onRetry: () => {},
    },
  });
  assert.equal(env.addedRows().length, 2);
  assert.equal(env.catalogRows().length, 2);
  assert.equal(env.control("Previous").props.disabled, true);
  assert.equal(env.control("Next").props.disabled, true);
  assert.equal(env.control("Reload skills").props.disabled, true);
  assert.equal(env.control("Retry skill catalog").props.disabled, true);
  env.addedRows()[0]!.props.onPress();
  env.control("Search skills").props.onChangeText("DNS");
  assert.deepEqual(
    env.calls.map((call) => call.kind),
    ["open", "query"],
  );
});

test("offline skill availability and reconnection guidance use the shared app translations", () => {
  for (const [language, expected] of [
    ["zh-CN", "离线时无法加载已添加的技能"],
    ["zh-TW", "離線時無法載入已新增的技能"],
  ] as const) {
    const env = setup(
      {
        offline: true,
        added: { items: [], known: false, loading: false, onRetry() {} },
        catalog: { items: [], known: false, loading: false, onRetry() {} },
      },
      language,
    );
    assert.ok(env.hasText(expected));
    assert.doesNotMatch(env.text(), /Reconnect|unavailable offline/);
  }
});
