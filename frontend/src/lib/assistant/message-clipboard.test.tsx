import { describe, expect, it } from "vitest";
import { messageClipboardContent } from "./message-clipboard";

function parsed(html: string) {
  const body = document.createElement("div");
  body.innerHTML = html;
  return body;
}

describe("message clipboard body", () => {
  it("copies the issue's Chinese sample as readable paragraphs and linked rich text", () => {
    const source = `我查一下木瓜果腔里这种白色组织的资料，结合照片判断。

从照片看，中间这块光滑、乳白色的东西，**更像木瓜自身长出的异常肉质组织，俗称“胎木瓜”或“瓜中瓜”**。有类似白色条状组织的报道，但仅凭照片还不能完全确定。([tw.news.yahoo.com](https://tw.news.yahoo.com/%E6%9C%A8%E7%93%9C%E9%95%B7%E5%87%BA-%E7%99%BD%E8%89%B2%E6%A2%9D%E7%8B%80%E7%89%A9-%E5%B0%88%E5%AE%B6%E8%A7%A3%E6%83%91-%E7%95%B0%E8%AE%8A%E5%BD%A2%E6%88%90%E7%9A%84-%E8%83%8E%E6%9C%A8%E7%93%9C-025458593.html?utm_source=openai))

纽约植物园也解释过，木瓜内部有时会出现比籽大很多的白色块状物，可能与种子在果实内部提前萌发等异常发育有关。你这块外观更接近完整的肉质组织。([libanswers.nybg.org](https://libanswers.nybg.org/faq/223478))

**这块先去掉，别急着吃。**照片无法确认整颗是否安全；如果还有长毛霉斑、明显腐烂或异常软烂，就整颗丢弃，不能只挖掉坏的部分——霉菌可能已进入柔软果肉内部。([ask.fsis.usda.gov](https://ask.fsis.usda.gov/article/What-if-I-find-moldy-fruit-in-a-container-with-other-fruit?utm_source=openai))`;
    const content = messageClipboardContent(source);
    expect(content.text).toBe(source.replaceAll("**", "").replace(/\[([^\]]+)\]\([^)]+\)/g, "$1"));
    const body = parsed(content.html);
    expect(body.querySelectorAll("p")).toHaveLength(4);
    expect([...body.querySelectorAll("strong")].map((node) => node.textContent)).toEqual([
      "更像木瓜自身长出的异常肉质组织，俗称“胎木瓜”或“瓜中瓜”",
      "这块先去掉，别急着吃。",
    ]);
    expect([...body.querySelectorAll("a")].map((node) => [node.textContent, node.getAttribute("href")])).toEqual([
      ["tw.news.yahoo.com", expect.stringContaining("%E6%9C%A8")],
      ["libanswers.nybg.org", "https://libanswers.nybg.org/faq/223478"],
      ["ask.fsis.usda.gov", expect.stringContaining("https://ask.fsis.usda.gov/article/")],
    ]);
    expect(content.text).not.toContain("https://");
  });

  it("retains literal HTML/code while enforcing the renderer's URL and image rules", () => {
    const content = messageClipboardContent([
      '<script>alert("literal")</script>',
      '<img src=x onerror="alert(1)">',
      "[unsafe](javascript:alert(1)) [http](http://example.test) [relative](/private) [email](mailto:hi@example.test)",
      "![remote](https://example.test/pixel.png)",
      "[![nested](https://example.test/tracker)](https://example.test/page)",
      "\\*\\*这块先去掉，别急着吃。\\*\\*照片",
      "`**literal** <TOKEN>`",
      "```ts\n  const a = '**not bold**';\n    const b = '<tag>';\n```",
    ].join("\n\n"));
    const body = parsed(content.html);
    expect(body.querySelector("script, img, input, button")).toBeNull();
    expect([...body.querySelectorAll("a")].map((node) => node.getAttribute("href"))).toEqual([
      "mailto:hi@example.test", "https://example.test/pixel.png", "https://example.test/page",
    ]);
    expect(body.querySelector("strong")).toBeNull();
    expect(content.text).toContain('<script>alert("literal")</script>');
    expect(content.text).toContain('<img src=x onerror="alert(1)">');
    expect(content.text).toContain("**这块先去掉，别急着吃。**照片");
    expect(content.text).toContain("**literal** <TOKEN>");
    expect(content.text).toContain("  const a = '**not bold**';\n    const b = '<tag>';");
    expect(content.text).toContain("remote\n\nnested");
  });

  it("uses the same validated hosted-link href and visible label", () => {
    const content = messageClipboardContent(`[GitHub](${window.location.origin}/channel-bots/connect/telegram?label=Work&token=secret)`);
    const link = parsed(content.html).querySelector("a");
    expect(link?.getAttribute("href")).not.toContain("token=");
    expect(content.text).toBe(link?.textContent);
    const hosted = messageClipboardContent(`[GitHub](${window.location.origin}/connect/nyx_clk_test)`);
    expect(hosted.text).toBe("Connect GitHub");
    expect(parsed(hosted.html).querySelector("a")?.getAttribute("href")).toBe(`${window.location.origin}/connect/nyx_clk_test`);
  });

  it("keeps list, table and break structure without copying hidden footnote headings or form controls", () => {
    const content = messageClipboardContent([
      "## Heading",
      "Paragraph with *emphasis* and ~~deleted~~ text.  \nNext line.",
      "3. Third\n4. Fourth",
      "- [x] Done\n- [ ] Pending",
      "| Name | Status |\n| --- | --- |\n| Agent | OK |",
      "A note[^1].\n\n[^1]: Footnote body",
    ].join("\n\n"));
    expect(content.text).toContain("Heading\n\nParagraph with emphasis and deleted text.\nNext line.");
    expect(content.text).toContain("3. Third\n4. Fourth");
    expect(content.text).toContain("• [x] Done\n• [ ] Pending");
    expect(content.text).toContain("Name\tStatus\nAgent\tOK");
    expect(content.text).not.toContain("FootnotesFootnotes");
    const body = parsed(content.html);
    expect(body.querySelector("input, .sr-only, [hidden], [aria-hidden]")).toBeNull();
    expect(body.querySelectorAll("h2")).toHaveLength(1);
    expect(body.querySelectorAll("em, del")).toHaveLength(2);
  });

  it("preserves nested list indentation, hard breaks and code trailing blank lines", () => {
    expect(messageClipboardContent("- Parent\n  - First child\n  - Second child\n- Next parent").text)
      .toBe("• Parent\n  • First child\n  • Second child\n• Next parent");
    expect(messageClipboardContent("First  \nSecond").text).toBe("First\nSecond");
    const code = "```text\n  value\n\n```";
    const result = messageClipboardContent(code);
    expect(result.text).toBe("  value\n\n");
    expect(parsed(result.html).querySelector("code")?.textContent).toBe(result.text);
  });

  it("copies footnotes as readable content without dangling fragment links", () => {
    const result = messageClipboardContent("Fact[^1].\n\n[^1]: Source note.");
    expect(result.text).toBe("Fact[1].\n\nFootnotes\n\n1. Source note.");
    expect(parsed(result.html).querySelector('a[href^="#"]')).toBeNull();
  });

  it("preserves paragraph spaces and inherited inline-code whitespace without changing headings or tight lists", () => {
    const result = messageClipboardContent("`  inline  code  ` and  text.\nNext  line.\n\n## A  heading\n\n- Tight  item");
    const body = parsed(result.html);
    const paragraph = body.querySelector("p")!;
    expect(paragraph.style.whiteSpace).toBe("pre-wrap");
    expect(paragraph.querySelector("code")?.textContent).toBe(" inline  code ");
    expect(paragraph.querySelector("code")?.style.whiteSpace).toBe("");
    expect(paragraph.querySelector("br")).not.toBeNull();
    expect(result.text).toContain(" inline  code  and  text.\nNext  line.");
    expect(body.querySelector("h2")?.style.whiteSpace).toBe("");
    expect(body.querySelector("li")?.style.whiteSpace).toBe("");
  });

  it("omits structural separator newlines before literal raw HTML", () => {
    expect(messageClipboardContent('**Answer**\n\n[source](https://example.test/%E4%B8%AD)\n\n<script>literal</script>').text)
      .toBe("Answer\n\nsource\n\n<script>literal</script>");
  });

  it.each(["**Alice**", "[Alice](https://example.test)"])("keeps a mention display name literal: %s", (label) => {
    const result = messageClipboardContent("Hello @alice and **team**. `@alice`", [{ handle: "alice", label }]);
    const body = parsed(result.html);
    expect(result.text).toBe(`Hello @${label} and team. @alice`);
    expect(body.querySelectorAll("strong")).toHaveLength(1);
    expect(body.querySelector("a")).toBeNull();
    expect(body.querySelector("span")?.textContent).toBe(`@${label}`);
  });
});
