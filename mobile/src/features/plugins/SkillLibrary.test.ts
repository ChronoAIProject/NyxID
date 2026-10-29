import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../assistant/assistantHookTestHarness";
import type { AddedSkillItem } from "./SkillLibrary";
import type { CatalogSkill } from "./pluginSkillsTypes";

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

function textContent(root: Element): string {
  return [root.props.children]
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

function setup(overrides: Record<string, unknown> = {}) {
  const calls: { kind: string; args: unknown[] }[] = [];
  const record =
    (kind: string) =>
    (...args: unknown[]) =>
      calls.push({ kind, args });
  const harness = hookHarness<Element>({
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
      ["BookOpen", "Check", "ChevronRight", "Search", "X"].map((name) => [
        name,
        name,
      ]),
    ),
    "../../components/workspace/WorkspaceUI": {
      ActionButton: "ActionButton",
      Label: "Label",
      Notice: "Notice",
    },
    "../../theme/designTokens": { fonts: {} },
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
        tab: "added",
        query: "",
        added: addedState,
        catalog: catalogState,
        page: 1,
        totalPages: 1,
        onTabChange: record("tab"),
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
  assert.match(env.text(), /Added to this agent/);
  assert.match(env.text(), /Added · 2/);
  assert.match(env.text(), /2 of 16/);
  assert.match(env.text(), /Cloudflare dns helper/);
  assert.match(env.text(), /Safely review and update DNS records/);
  assert.match(env.text(), /Version 2026\.10-exactEnabled/);
  assert.match(env.text(), /Weekly report/);
  assert.match(env.text(), /Version 1\.0\.3Disabled/);
  assert.equal(env.rows().length, 2);
  assert.equal(
    env.control("Added skills").props.accessibilityState.selected,
    true,
  );
  assert.equal(
    env.control("Discover skills").props.accessibilityState.selected,
    false,
  );
});

test("Added searches names, descriptions and slug words using spaces, case and repeated whitespace", () => {
  for (const query of [
    "CloudFlare DNS",
    "cloudflare-dns",
    "  CLOUDFLARE   DNS  ",
    "review and update dns",
  ]) {
    const env = setup({ query });
    assert.equal(env.rows().length, 1, query);
    assert.match(
      env.rows()[0]!.props.accessibilityLabel,
      /^cloudflare-dns-helper,/,
    );
    assert.match(env.text(), /Added · 2/);
    assert.match(env.text(), /2 of 16/);
  }
  assert.match(
    setup({ query: "weekly report" }).rows()[0]!.props.accessibilityLabel,
    /^weekly_report,/,
  );
  assert.match(
    setup({ query: "sales figures" }).rows()[0]!.props.accessibilityLabel,
    /^weekly_report,/,
  );
});

test("clear search and tabs expose their actual scope without attaching or changing any skill", () => {
  const added = setup({ query: "DNS" });
  added.control("Clear skill search").props.onPress();
  added.control("Discover skills").props.onPress();
  assert.deepEqual(added.calls, [
    { kind: "query", args: [""] },
    { kind: "keyboard", args: [] },
    { kind: "tab", args: ["discover"] },
  ]);
  const discover = setup({ tab: "discover", query: "cloud" });
  const input = discover.control("Search skill catalog");
  assert.equal(input.props.value, "cloud");
  assert.equal(input.props.maxLength, 200);
  input.props.onChangeText("another");
  discover.control("Added skills").props.onPress();
  assert.deepEqual(discover.calls, [
    { kind: "query", args: ["another"] },
    { kind: "keyboard", args: [] },
    { kind: "tab", args: ["added"] },
  ]);
  assert.throws(() => discover.control("Search added skills"));
});

test("catalog opens read-only review with the original opaque id and exact name, never the display label", () => {
  const env = setup({ tab: "discover", query: "keyword" });
  assert.equal(
    env.rows().length,
    2,
    "catalog results are server-filtered, not filtered a second time by Added search",
  );
  assert.match(env.text(), /Find a skill for Research agent/);
  assert.match(
    env.rows()[0]!.props.accessibilityLabel,
    /^weekly_report, Added$/,
  );
  assert.match(
    env.rows()[1]!.props.accessibilityLabel,
    /^Document Expert, View versions$/,
  );
  assert.match(textContent(env.rows()[0]!), /Weekly report/);
  env.rows()[0]!.props.onPress();
  env.rows()[1]!.props.onPress();
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
  added.control("Loading added skills");
  assert.equal(added.hasText("No skills added yet"), false);
  assert.equal(added.hasText("Added · 0"), false);
  assert.equal(added.hasText("0 of 16"), false);
  const discover = setup({ tab: "discover", catalog: state });
  discover.control("Searching skill catalog");
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
    tab: "discover",
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
  const discover = elements(added.tree).find(
    (item) =>
      item.type === "ActionButton" && item.props.label === "Discover skills",
  )!;
  discover.props.onPress();
  assert.deepEqual(added.calls, [
    { kind: "keyboard", args: [] },
    { kind: "tab", args: ["discover"] },
  ]);
  const noMatch = setup({ query: "no such workflow" });
  assert.match(noMatch.text(), /No matching added skills/);
  assert.equal(noMatch.hasText("No skills added yet"), false);
  const catalogEmpty = setup({
    tab: "discover",
    catalog: { items: [], known: true, loading: false, onRetry: () => {} },
  });
  assert.match(catalogEmpty.text(), /No skills found/);
  assert.match(
    catalogEmpty.text(),
    /Only skills available through your Ornn access appear here/,
  );
});

test("catalog pagination has explicit boundaries, busy state and no Added pagination", () => {
  const env = setup({ tab: "discover", page: 2, totalPages: 4 });
  assert.match(env.text(), /2 \/ 4/);
  env.control("Previous").props.onPress();
  env.control("Next").props.onPress();
  assert.deepEqual(env.calls, [
    { kind: "page", args: [1] },
    { kind: "page", args: [3] },
  ]);
  assert.equal(
    setup({ tab: "discover", page: 1, totalPages: 4 }).control("Previous").props
      .disabled,
    true,
  );
  assert.equal(
    setup({ tab: "discover", page: 4, totalPages: 4 }).control("Next").props
      .disabled,
    true,
  );
  const busy = setup({
    tab: "discover",
    page: 2,
    totalPages: 4,
    catalog: { items: catalog, known: true, loading: true, onRetry: () => {} },
  });
  assert.equal(busy.control("Previous").props.disabled, true);
  assert.equal(busy.control("Next").props.disabled, true);
  assert.equal(
    busy.rows().length,
    2,
    "cached results remain readable during refresh",
  );
  assert.throws(() => setup({ totalPages: 4 }).control("Next"));
});

test("blank descriptions offer review guidance rather than another opaque slug", () => {
  const env = setup({
    tab: "discover",
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
