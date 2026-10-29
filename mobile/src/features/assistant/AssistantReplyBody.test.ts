import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "./assistantHookTestHarness";

test("private, streaming and crew reply cards are separated from prose without adding start/end margins", () => {
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: any, props: any) => ({ type, props }),
      jsxs: (type: any, props: any) => ({ type, props }),
    },
    "react-native": { View: "View" },
    "./AssistantMarkdownBody": { AssistantMarkdownBody: "Markdown" },
  });
  const Body = (
    h.load(fileURLToPath(new URL("./AssistantReplyBody.tsx", import.meta.url)))
      .AssistantReplyBody as any
  ).type;
  for (const variant of ["private", "crew"]) {
    h.mount(() => Body({ text: "## Review", activity: "Receipt", variant }));
    const body = h.value();
    assert.equal(body.props.style.gap, 16);
    assert.equal(body.props.children[0].props.style.gap, 8);
    assert.equal(body.props.children[1].props.variant, variant);
    assert.equal(body.props.style.marginTop, undefined);
    assert.equal(body.props.style.marginBottom, undefined);
    h.unmount();
    h.mount(() => Body({ text: "Plain reply", variant }));
    assert.equal(
      h.value().props.children.filter(Boolean).length,
      1,
      "no empty activity wrapper before the first block",
    );
    h.unmount();
  }
});
