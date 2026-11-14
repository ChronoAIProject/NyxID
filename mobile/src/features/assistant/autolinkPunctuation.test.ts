import assert from "node:assert/strict";
import test from "node:test";
import MarkdownIt from "markdown-it";
import fixtures from "../../../../shared/autolink-punctuation.json";
import {
  configureAssistantMarkdown,
  safeAssistantLinkUrl,
} from "./assistantMarkdown";
import { assistantLinkTarget } from "./assistantLinkTarget";
import { bareAutolinkBoundary } from "../../../../shared/autolinkPunctuation";
const parser = configureAssistantMarkdown(new MarkdownIt({ linkify: true }));
for (const fixture of fixtures)
  test(`bare autolink boundary: ${fixture.text}`, () => {
    const tokens = parser
      .parse(fixture.text, {})
      .flatMap((token) => token.children ?? []);
    const links = tokens.filter((token) => token.type === "link_open");
    assert.deepEqual(
      links.map((token) => token.attrGet("href")),
      "mobileHrefs" in fixture ? fixture.mobileHrefs : fixture.hrefs,
    );
    assert.ok(
      links.every(
        (token) => token.attrGet("data-nyx-bare-autolink") === "true",
      ),
    );
    assert.equal(
      tokens
        .filter((token) => token.type === "text")
        .map((token) => token.content)
        .join(""),
      fixture.text,
    );
    // ASCII punctuation is bounded by markdown-it; the open-path defence handles CJK.
    if (
      fixture.hrefs.length === 1 &&
      fixture.suffix &&
      /[。]/u.test(fixture.suffix)
    )
      assert.equal(
        safeAssistantLinkUrl(
          fixture.hrefs[0] + fixture.suffix,
          ["https://nyxid.ai"],
          true,
        ),
        fixture.hrefs[0],
      );
  });
test("explicit Markdown and angle URLs preserve authored punctuation", () => {
  const tokens = parser
    .parse("[label](https://x.y/z。) <https://x.y/z。>", {})
    .flatMap((token) => token.children ?? []);
  assert.deepEqual(
    tokens
      .filter((token) => token.type === "link_open")
      .map((token) => decodeURI(token.attrGet("href")!)),
    ["https://x.y/z。", "https://x.y/z。"],
  );
});

test("link-open defence bounds only links with bare-autolink provenance", () => {
  const url = "https://nyxid.ai/connect/nyx_clk_example。";
  assert.equal(
    assistantLinkTarget(url, "https://nyxid.ai", true)?.href,
    "https://nyxid.ai/connect/nyx_clk_example",
  );
  assert.equal(assistantLinkTarget(url, "https://nyxid.ai"), null);
  assert.equal(
    safeAssistantLinkUrl(url, "https://nyxid.ai"),
    new URL(url).href,
  );
});

test("CJK suffixes preserve every fixture's link safety outcome, including schemeless literals", () => {
  for (const fixture of fixtures) {
    for (const suffix of [
      "。",
      "，",
      "、",
      "！",
      "？",
      "；",
      "：",
      "）",
      "」",
      "』",
      "】",
      "》",
      "”",
    ]) {
      const text = fixture.text + suffix;
      const tokens = parser
        .parse(text, {})
        .flatMap((token) => token.children ?? []);
      assert.deepEqual(
        tokens
          .filter((token) => token.type === "link_open")
          .map((token) => token.attrGet("href")),
        "mobileHrefs" in fixture ? fixture.mobileHrefs : fixture.hrefs,
        text,
      );
      assert.equal(
        tokens
          .filter((token) => token.type === "text")
          .map((token) => token.content)
          .join(""),
        text,
      );
    }
  }
});

test("only trailing unmatched delimiters leave a URL, never interior apostrophes, quotes or parentheses", () => {
  for (const url of [
    "https://en.wikipedia.org/wiki/Ender’s_Game",
    "https://example.com/a”b",
    "https://example.com/a)b",
  ]) {
    assert.equal(bareAutolinkBoundary(url), url.length);
    assert.equal(bareAutolinkBoundary(url + "。"), url.length);
  }
});
