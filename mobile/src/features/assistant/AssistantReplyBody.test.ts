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
  for (const text of ["## Review", "Plain reply"]) {
    h.mount(() => Body({ text, activity: "Receipt" }));
    const body = h.value();
    assert.equal(body.props.style.gap, 16);
    assert.equal(body.props.children[0].props.style.gap, 8);
    assert.equal(body.props.children[1].props.variant, undefined);
    assert.equal(body.props.style.marginTop, undefined);
    assert.equal(body.props.style.marginBottom, undefined);
    h.unmount();
    h.mount(() => Body({ text }));
    assert.equal(
      h.value().props.children.filter(Boolean).length,
      1,
      "no empty activity wrapper before the first block",
    );
    h.unmount();
  }
});

test("reply body emits the text marker at layout commit only when it renders markdown", () => {
  const h = hookHarness<any>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": { View: "View" },
    "./AssistantMarkdownBody": { AssistantMarkdownBody: "Markdown" },
  });
  const Body = (
    h.load(fileURLToPath(new URL("./AssistantReplyBody.tsx", import.meta.url)))
      .AssistantReplyBody as any
  ).type;
  let commits = 0;
  const onTextCommit = () => {
    commits++;
  };
  const empty = Body({ text: "", activity: "Receipt", onTextCommit });
  assert.equal(empty.props.children.filter(Boolean).length, 1);
  const rendered = Body({ text: "Hello", onTextCommit });
  const [markdown, marker] = rendered.props.children.filter(Boolean);
  assert.equal(markdown.type, "Markdown");
  assert.equal(commits, 0);
  h.mount(() => marker.type(marker.props));
  assert.equal(commits, 1);
  h.render();
  assert.equal(commits, 1);
  h.unmount();
});
