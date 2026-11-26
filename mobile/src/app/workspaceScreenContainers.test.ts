import assert from "node:assert/strict";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";

/*
 * The folded Duo's floating rail keeps readable content clear through one
 * place: the shared page containers add its trailing inset (WorkspaceUI and
 * ScreenContainer tests cover the containers themselves). This structural
 * check keeps every page on the workspace content slab inside one of them, so
 * a new page cannot silently put text under the glass.
 */

const SRC = fileURLToPath(new URL("..", import.meta.url));
const CONTAINER = /<(WorkspacePage|ScreenContainer|SettingsPage)\b/;

function resolveModule(fromFile: string, spec: string) {
  for (const ext of [".tsx", ".ts", "/index.tsx", "/index.ts"]) {
    const path = join(dirname(fromFile), spec + ext);
    if (existsSync(path)) return path;
  }
  return null;
}

/** Relative named imports: local name -> resolved file. */
function namedImports(source: string, file: string) {
  const map = new Map<string, string>();
  for (const [, names, spec] of source.matchAll(
    /import\s*(?:type\s*)?\{([^}]*)\}\s*from\s*"([^"]+)"/g,
  )) {
    if (!spec!.startsWith(".")) continue;
    const target = resolveModule(file, spec!);
    if (!target) continue;
    for (const raw of names!.split(",")) {
      const part = raw.trim().replace(/^type\s+/, "");
      if (!part) continue;
      const [original, alias] = part.split(/\s+as\s+/);
      map.set((alias ?? original)!.trim(), target);
    }
  }
  return map;
}

/** A top-level component's source, up to the next top-level component. */
function componentBody(source: string, name: string) {
  const start = new RegExp(
    `(?:^|\\n)(?:export\\s+)?(?:function\\s+${name}\\b|const\\s+${name}\\s*=)`,
  ).exec(source);
  if (!start) return null;
  const rest = source.slice(start.index + 1);
  const next = rest.search(/\n(?:export\s+)?(?:function|const)\s+[A-Z]/);
  return next < 0 ? rest : rest.slice(0, next);
}

/**
 * Follows a screen component through route wrappers (at most `depth` JSX
 * hops) to the component that renders a shared page container.
 */
function containerPath(
  name: string,
  file: string,
  depth: number,
  seen = new Set<string>(),
): string[] | null {
  const source = readFileSync(file, "utf8");
  const body = componentBody(source, name);
  if (body === null) return null;
  const here = `${relative(SRC, file)}:${name}`;
  if (CONTAINER.test(body)) return [here];
  if (depth === 0) return null;
  const imports = namedImports(source, file);
  for (const [, tag] of body.matchAll(/<([A-Z]\w*)/g)) {
    const key = `${file}:${tag}`;
    if (tag === name || seen.has(key)) continue;
    seen.add(key);
    const target =
      imports.get(tag!) ?? (componentBody(source, tag!) ? file : null);
    if (!target) continue;
    const path = containerPath(tag!, target, depth - 1, seen);
    if (path) return [here, ...path];
  }
  return null;
}

test("every workspace content page renders inside a container that keeps text clear of the floating rail", () => {
  const navigator = join(SRC, "app/AppNavigator.tsx");
  const source = readFileSync(navigator, "utf8");
  const workspace = componentBody(source, "WorkspaceNavigator")!;
  const screens = [
    ...workspace.matchAll(
      /<ContentStack\.Screen\s+name="(\w+)"\s+component=\{(\w+)\}/g,
    ),
  ].map(([, route, component]) => ({ route: route!, component: component! }));
  // The parser saw every content route (none written in another shape).
  assert.equal(
    screens.length,
    source.match(/<ContentStack\.Screen\b/g)!.length,
    "every ContentStack.Screen is declared as name=… component={…}",
  );
  assert.ok(screens.length >= 40, `found ${screens.length} content routes`);
  assert.ok(screens.some((s) => s.route === "Assistant"));
  assert.ok(screens.some((s) => s.route === "Notifications"));
  const imports = namedImports(source, navigator);
  const missing = screens.filter(({ component }) => {
    const file =
      imports.get(component) ??
      (componentBody(source, component) ? navigator : null);
    return !file || !containerPath(component, file, 3);
  });
  assert.deepEqual(
    missing.map((s) => `${s.route} (${s.component})`),
    [],
    "pages outside WorkspacePage / ScreenContainer / SettingsPage",
  );
});

test("Settings pages reach the rail inset through ScreenContainer", () => {
  const file = join(SRC, "components/settings/SettingsList.tsx");
  const body = componentBody(readFileSync(file, "utf8"), "SettingsPage")!;
  assert.match(body, /<ScreenContainer\b/);
});

test("chat composers live inside their page, so they get the rail inset too", () => {
  for (const [path, component] of [
    ["features/assistant/AssistantScreen.tsx", "AssistantScreen"],
    ["features/groups/GroupChatScreen.tsx", "GroupChat"],
  ] as const) {
    const body = componentBody(readFileSync(join(SRC, path), "utf8"), component)!;
    const open = body.indexOf("<WorkspacePage");
    const close = body.lastIndexOf("</WorkspacePage>");
    const dock = body.indexOf("<ChatComposerDock");
    assert.ok(open >= 0 && close > open, `${component} renders a WorkspacePage`);
    assert.ok(dock > open && dock < close, `${component}'s composer is inside it`);
  }
});

/*
 * Vertical scroll indicators: every vertical scroller that can sit beside the
 * folded Duo's rail spreads `useRailScrollIndicator()` so its
 * indicator stays between the content and the glass. Others are horizontal,
 * hide the indicator, forward the prop from their caller (`{...props}`), or
 * render inside a <Modal> (its own window covers the rail). Whole files are
 * exempt only when they never render beside the rail.
 */
const SCROLL_EXEMPT_FILES: Record<string, string> = {
  "features/auth/AuthHomeScreen.tsx": "signed out: no workspace, no rail",
  "features/auth/AgentKeyLoginScreen.tsx": "root-stack screen covering the workspace",
  "features/auth/DeviceLoginScreen.tsx": "root-stack screen covering the workspace",
  "features/account/AccountDeletionSheet.tsx": "Settings sheet covers the rail",
  "components/workspace/WorkspaceChatList.tsx": "drawer and agent-picker sheet cover the rail",
};

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory()
      ? sourceFiles(path)
      : /\.tsx$/.test(entry.name) && !/(\.test\.|Harness)/.test(entry.name)
        ? [path]
        : [];
  });
}

/** The JSX props of the element opening at `start` (up to its first `>`). */
function openingProps(source: string, start: number) {
  let depth = 0;
  for (let i = start; i < source.length; i++) {
    const c = source[i];
    if (c === "{" || c === "(") depth++;
    else if (c === "}" || c === ")") depth--;
    else if (c === ">" && depth === 0) return source.slice(start, i);
  }
  return source.slice(start);
}

test("vertical scroll indicators keep clear of the folded Duo's rail", () => {
  const missing: string[] = [];
  let checked = 0;
  for (const file of sourceFiles(SRC)) {
    const name = relative(SRC, file);
    if (SCROLL_EXEMPT_FILES[name]) continue;
    const source = readFileSync(file, "utf8");
    for (const match of source.matchAll(
      /(?<![\w.])<((?:Animated\.)?(?:FlatList|SectionList|ScrollView)|PageScrollView)\b/g,
    )) {
      const props = openingProps(source, match.index! + match[0].length);
      const before = source.slice(0, match.index);
      const inModal =
        (before.match(/<Modal\b/g)?.length ?? 0) >
        (before.match(/<\/Modal>/g)?.length ?? 0);
      if (
        inModal ||
        /\bhorizontal\b(?!\s*=\s*\{false\})/.test(props) ||
        /showsVerticalScrollIndicator=\{false\}/.test(props) ||
        // Forwarding wrappers (PageScrollView): their callers are checked.
        /\{\.\.\.(?:props|scrollProps)\}/.test(props)
      )
        continue;
      checked++;
      if (!/\{\.\.\.railIndicator\}/.test(props))
        missing.push(`${name}:${before.split("\n").length} <${match[1]}>`);
    }
  }
  assert.ok(checked >= 20, `checked ${checked} vertical scrollers`);
  assert.deepEqual(missing, [], "spread useRailScrollIndicator()");
  for (const name of Object.keys(SCROLL_EXEMPT_FILES))
    assert.ok(existsSync(join(SRC, name)), `${name} still exists`);
});

test("scrollers take their indicator insets from the shared rail hook", () => {
  for (const file of sourceFiles(SRC)) {
    const source = readFileSync(file, "utf8");
    if (!/\{\.\.\.railIndicator\}/.test(source)) continue;
    assert.match(
      source,
      /const railIndicator = useRailScrollIndicator\((?:"page")?\);/,
      relative(SRC, file),
    );
  }
});
