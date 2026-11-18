import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";

type E = { type: any; props: Record<string, any> };
const nodes = (n: any): E[] =>
  n?.props ? [n, ...[n.props.children].flat(Infinity).flatMap(nodes)] : [];

function render(props: Record<string, unknown>) {
  const harness = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Text: "Text",
      StyleSheet: { create: (value: unknown) => value },
    },
    "react-native-svg": {
      __esModule: true,
      default: "Svg",
      Circle: "Circle",
      SvgXml: "SvgXml",
    },
    "../theme/ThemeContext": {
      useTheme: () => ({
        colors: { success: "green", textMuted: "grey", border: "line" },
      }),
    },
    "./brand/brandVectors": { nyxidIconXml: "<svg/>" },
    "./ServiceIcon": { ServiceIcon: "ServiceIcon" },
  });
  const { ConnectionIdentityHeading } = harness.load(
    fileURLToPath(new URL("./ConnectionIdentityHeading.tsx", import.meta.url)),
  ) as { ConnectionIdentityHeading: (props: object) => any };
  return nodes(ConnectionIdentityHeading(props));
}

test("connection heading pairs NyxID and the service like the web connector page", () => {
  const all = render({
    serviceSlug: "api-github",
    serviceName: "GitHub",
    title: "NyxID wants to connect to your GitHub",
    subtitle: "Add your connection details and approve the request.",
  });
  assert.ok(
    all.some((n) => n.props.accessibilityLabel === "NyxID connects to GitHub"),
  );
  assert.equal(
    all.find((n) => n.type === "ServiceIcon")!.props.slug,
    "api-github",
  );
  assert.equal(all.find((n) => n.type === "SvgXml")!.props.xml, "<svg/>");
  const dots = all.filter((n) => n.type === "Circle");
  assert.equal(dots.length, 11);
  assert.ok(dots.every((dot) => dot.props.fill === "grey"));
  const header = all.find((n) => n.props.accessibilityRole === "header")!;
  assert.equal(header.props.children, "NyxID wants to connect to your GitHub");
  const done = render({
    serviceSlug: "api-github",
    serviceName: "GitHub",
    title: "GitHub connected",
    complete: true,
  });
  assert.ok(
    done
      .filter((n) => n.type === "Circle")
      .every((dot) => dot.props.fill === "green"),
  );
});
