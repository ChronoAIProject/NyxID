import assert from "node:assert/strict";
import test from "node:test";
import { resolveLegalLink } from "./legalLink";

const documentUrl = "https://nyxid.example/legal/privacy.md";

test("canonical legal links resolve to native document routes", () => {
  for (const docKey of ["terms", "privacy"] as const) {
    for (const href of [
      `/${docKey}`,
      `/${docKey}?from=legal#intro`,
      `https://nyxid.example/${docKey}`,
    ]) {
      assert.deepEqual(resolveLegalLink(href, documentUrl), {
        kind: "document",
        docKey,
      });
    }
    assert.deepEqual(
      resolveLegalLink(`/${docKey}`, "http://localhost:3000/legal/terms.md"),
      { kind: "document", docKey },
    );
  }
});

test("external HTTPS and mail links remain external, including same-named paths on other origins", () => {
  for (const href of [
    "https://other.example/privacy",
    "https://nyxid.example/terms-extra",
    "mailto:contact@chrono-ai.fun",
  ]) {
    assert.deepEqual(resolveLegalLink(href, documentUrl), {
      kind: "external",
      url: href,
    });
  }
  assert.deepEqual(resolveLegalLink("../help", documentUrl), {
    kind: "external",
    url: "https://nyxid.example/help",
  });
  assert.deepEqual(resolveLegalLink("https://other.example/terms", null), {
    kind: "external",
    url: "https://other.example/terms",
  });
});

test("malformed links, unsafe schemes and embedded credentials never reach native Linking", () => {
  for (const href of [
    "javascript:alert(1)",
    "data:text/html,test",
    "file:///etc/passwd",
    "nyxid://auth",
    "http://other.example/privacy",
    "https://user:password@nyxid.example/terms",
    "https://[bad",
    "mailto://user:password@example.com",
  ]) {
    assert.deepEqual(
      resolveLegalLink(href, documentUrl),
      { kind: "unsupported" },
      href,
    );
  }
  assert.deepEqual(resolveLegalLink("/terms", null), { kind: "unsupported" });
});
