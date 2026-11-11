import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { URL, fileURLToPath } from "node:url";
import { runInNewContext } from "node:vm";
import test from "node:test";
import ts from "typescript";

type Element = { type: any; props: Record<string, any> };
const jsx = (type: any, props: any) => ({ type, props });
const colors = {
  card: "#fff",
  bg: "#fff",
  border: "#ddd",
  textPrimary: "#111",
};
function registry(filename: string) {
  const cache = new Map<string, any>();
  function load(filename: string): any {
    if (cache.has(filename)) return cache.get(filename);
    const exports = {};
    cache.set(filename, exports);
    const nativeRequire = createRequire(filename);
    const output = ts.transpileModule(readFileSync(filename, "utf8"), {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
        jsx: ts.JsxEmit.ReactJSX,
        esModuleInterop: true,
      },
    }).outputText;
    runInNewContext(
      output,
      {
        exports,
        require: (name: string) => {
          if (name === "react/jsx-runtime") return { jsx, jsxs: jsx };
          if (name === "react")
            return {
              cloneElement: (el: Element, props: any) =>
                jsx(el.type, { ...el.props, ...props }),
            };
          if (name === "react-native") return { View: "View" };
          if (name === "react-native-svg")
            return {
              __esModule: true,
              default: "svg",
              Path: "path",
              Rect: "rect",
              Polygon: "polygon",
              G: "g",
            };
          if (name === "lucide-react" || name === "lucide-react-native")
            return new Proxy({}, { get: (_, key) => `lucide:${String(key)}` });
          if (name.endsWith("theme/ThemeContext"))
            return { useTheme: () => ({ colors }) };
          if (name.startsWith("."))
            return load(
              resolve(
                dirname(filename),
                name + (name.endsWith(".tsx") ? "" : ".tsx"),
              ),
            );
          return nativeRequire(name);
        },
      },
      { filename },
    );
    return exports;
  }
  return load(filename);
}
function flatten(el: any): Element[] {
  if (!el || typeof el !== "object" || !el.props) return [];
  if (typeof el.type === "function") return flatten(el.type(el.props));
  return [el, ...[el.props.children].flat(Infinity).flatMap(flatten)];
}
const nativePath = fileURLToPath(new URL("./index.tsx", import.meta.url));
const webPath = fileURLToPath(
  new URL(
    "../../../../frontend/src/components/service-icons/index.tsx",
    import.meta.url,
  ),
);
const native = registry(nativePath),
  web = registry(webPath);

test("every web registry key, explicit alias and prefix alias exists natively and names the same glyph", () => {
  assert.deepEqual(
    Object.keys(native.SERVICE_ICONS).sort(),
    Object.keys(web.SERVICE_ICONS).sort(),
  );
  assert.deepEqual(
    Array.from(native.SPEC_CATALOG_SLUGS),
    Array.from(web.SPEC_CATALOG_SLUGS),
  );
  assert.deepEqual(
    Array.from(native.CUSTOM_SERVICE_SLUGS),
    Array.from(web.CUSTOM_SERVICE_SLUGS),
  );
  for (const key of Object.keys(web.SERVICE_ICONS)) {
    const Native = native.SERVICE_ICONS[key],
      Web = web.SERVICE_ICONS[key];
    assert.equal(Native.name, Web.name, key);
    // Compare painted geometry, not just matching names or a placeholder port.
    const geometry = (Component: any) =>
      flatten(jsx(Component, { size: 24, color: "#444" }))
        .filter((e) => ["path", "rect", "polygon", "svg"].includes(e.type))
        .map((e) =>
          JSON.stringify(
            [
              e.type,
              e.props.d ?? null,
              e.props.points ?? null,
              e.props.viewBox ?? null,
              e.props.fill ?? null,
              e.props.stroke ?? null,
              e.props.x ?? null,
              e.props.y ?? null,
              e.props.width ?? null,
              e.props.height ?? null,
              e.props.rx ?? null,
              e.props.fillRule ?? null,
              e.props.clipRule ?? null,
              e.props.strokeWidth ?? null,
              e.props.strokeLinecap ?? null,
              e.props.strokeLinejoin ?? null,
            ].slice(0, e.type === "svg" ? 4 : undefined),
          ),
        );
    assert.deepEqual(geometry(Native), geometry(Web), key);
  }
});
test("composite marks retain their badge and requested native glyph size without clipping", () => {
  for (const key of [
    "api-github-pat",
    "api-google-workspace",
    "llm-google-ai",
    "chrono-llm",
    "llm-nyx",
  ]) {
    const tree = flatten(
      jsx(native.SERVICE_ICONS[key], { size: 32, color: "#444" }),
    );
    assert.ok(
      tree.some((e) => String(e.type).startsWith("lucide:")),
      key,
    );
    assert.ok(
      tree.some((e) => e.type === "svg" && e.props.height === 32),
      key,
    );
    assert.ok(
      tree.every((e) => e.props.style?.overflow !== "hidden"),
      key,
    );
  }
});
