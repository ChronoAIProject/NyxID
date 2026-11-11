import assert from "node:assert/strict";
import { URL, fileURLToPath } from "node:url";
import test from "node:test";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

type Element = { type: any; key?: string; props: Record<string, any> };
function setup(iconUrl?: string | null, slug: string | null = "api-github") {
  const harness = hookHarness<Element | null>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown, key?: string) => ({
        type,
        props,
        key,
      }),
    },
    "react-native": { Image: "Image", View: "View" },
    "react-native-svg": { SvgUri: "SvgUri" },
    "../theme/ThemeContext": {
      useTheme: () => ({ colors: { textSecondary: "#444" } }),
    },
    "./service-icons": {
      SERVICE_ICONS: { "api-github": "GitHub", github: "GitHub" },
      FallbackIcon: "Globe",
    },
  });
  const exports = harness.load(
    fileURLToPath(new URL("./ServiceIcon.tsx", import.meta.url)),
  ) as {
    ServiceIcon: (props: unknown) => Element | null;
    serviceIconSource: (value: unknown) => { svg: boolean; uri: string } | null;
    SERVICE_ICON_SIZES: Record<string, number>;
  };
  const props = { iconUrl, slug, size: "md", color: "green" };
  harness.mount(() => exports.ServiceIcon(props));
  return {
    harness,
    props,
    source: exports.serviceIconSource,
    sizes: exports.SERVICE_ICON_SIZES,
  };
}
function remote(view: ReturnType<typeof setup>) {
  const child = view.harness.value()!.props.children;
  view.harness.mount(() => child.type(child.props));
  return view.harness.value()!;
}

test("safe remote image takes precedence over a known glyph and uses public unauthenticated SVG loading", () => {
  const url =
    "https://id.example/catalog-icons/60badfba-533c-4305-b9bb-c545186b06d0";
  const view = setup(url);
  const svg = remote(view);
  assert.equal(svg.type, "SvgUri");
  assert.equal(svg.props.uri, url);
  assert.equal(svg.props.width, 24);
  assert.equal(svg.props.height, 24);
  assert.equal(svg.props.headers, undefined);
  assert.equal(svg.props.fallback.type, "GitHub");
  assert.equal(
    view.harness.render()!.props.onError,
    svg.props.onError,
    "rerender must not re-fetch SvgUri by changing its onError dependency",
  );
  svg.props.onError();
  assert.equal(view.harness.flush()!.type, "GitHub");
});
test("opaque external SVG recovers from image decoding failure, then falls back to its glyph on SVG error", () => {
  const view = setup("https://assets.example/icon?id=123");
  const image = remote(view);
  assert.equal(image.type, "Image");
  assert.equal(image.props.source.headers, undefined);
  assert.equal(image.props.resizeMode, "contain");
  image.props.onError();
  const svg = view.harness.flush()!;
  assert.equal(svg.type, "SvgUri");
  svg.props.onError();
  assert.equal(view.harness.flush()!.type, "GitHub");
});
test("remote failure with an unknown slug uses the globe", () => {
  const view = setup("https://assets.example/icon.svg", "unknown");
  const svg = remote(view);
  svg.props.onError();
  assert.equal(view.harness.flush()!.type, "Globe");
});
test("URL policy matches web: http/https only, no credentials or fragments, at most 2048 characters", () => {
  const view = setup();
  for (const value of [
    undefined,
    null,
    "",
    "not a URL",
    "/catalog-icons/id",
    "//assets.example/icon.svg",
    "file:///private/icon.svg",
    "data:image/svg+xml,<svg/>",
    "javascript:alert(1)",
    "https://user:password@assets.example/icon.png",
    "https://user@assets.example/icon.png",
    "https://assets.example/icon.svg#",
    `https://assets.example/${"a".repeat(2048)}`,
  ])
    assert.equal(view.source(value), null, String(value));
  const prefix = "https://assets.example/";
  assert.ok(view.source(prefix + "a".repeat(2048 - prefix.length)));
  assert.equal(view.source(prefix + "a".repeat(2049 - prefix.length)), null);
  assert.equal(view.source("http://localhost/catalog-icons/id")!.svg, true);
  assert.equal(view.source("https://assets.example/logo.SVG?v=2")!.svg, true);
  assert.equal(view.source("https://assets.example/logo.png")!.svg, false);
});
test("known slug/alias resolves the brand, unknown resolves globe, absent slug and URL renders nothing", () => {
  for (const slug of [
    "api-github",
    "github",
    "unknown",
    "constructor",
    "toString",
    "__proto__",
  ]) {
    const view = setup(null, slug);
    const box = view.harness.value()!;
    assert.equal(
      box.props.children.type,
      slug === "api-github" || slug === "github" ? "GitHub" : "Globe",
    );
    assert.equal(box.props.accessible, false);
    assert.equal(box.props.accessibilityElementsHidden, true);
    assert.equal(box.props.importantForAccessibility, "no-hide-descendants");
  }
  assert.equal(setup(null, null).harness.value(), null);
  assert.equal(
    setup("file:///icon.svg").harness.value()!.props.children.type,
    "GitHub",
  );
});
test("URL changes remount the failed loader and size tokens match the web scale", () => {
  const view = setup("https://assets.example/first.svg");
  const first = view.harness.value()!.props.children;
  view.props.iconUrl = "https://assets.example/second.svg";
  const second = view.harness.render()!.props.children;
  assert.notEqual(first.key, second.key);
  assert.equal(second.props.source.uri, view.props.iconUrl);
  assert.deepEqual(
    { ...view.sizes },
    { "2xs": 14, xs: 16, sm: 20, md: 24, lg: 32, xl: 36 },
  );
});
