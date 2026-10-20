import { Window } from "happy-dom";
export function semanticMarkdown(html, meta, revision, origin) {
  const window = new Window();
  const doc = window.document;
  doc.body.innerHTML = html;
  const main = doc.querySelector("main");
  if (!main) throw new Error(`Missing semantic main: ${meta.path}`);
  main
    .querySelectorAll(
      'script,style,svg,[aria-hidden="true"],[hidden],button,input,select,textarea',
    )
    .forEach((el) => el.remove());
  function text(node) {
    if (node.nodeType === 3) return node.textContent.replace(/\s+/g, " ");
    if (node.nodeType !== 1) return "";
    const inner = Array.from(node.childNodes, text).join(""),
      tag = node.localName;
    if (/^h[1-6]$/.test(tag))
      return `\n\n${"#".repeat(Number(tag[1]))} ${inner}\n\n`;
    if (tag === "img")
      return node.getAttribute("alt")
        ? `![${node.getAttribute("alt")}](${new URL(node.getAttribute("src"), origin + meta.path).href})`
        : "";
    if (tag === "code" && node.parentElement?.localName !== "pre")
      return "`" + inner + "`";
    if (tag === "table") {
      const rows = Array.from(node.querySelectorAll("tr"), (row) =>
        Array.from(row.children, (cell) =>
          text(cell).trim().replace(/\|/g, "\\|").replace(/\n+/g, "<br>"),
        ).join(" | "),
      );
      const columns = node.querySelector("tr")?.children.length ?? 0;
      return (
        `\n\n| ${rows[0]} |\n| ${Array(columns).fill("---").join(" | ")} |\n` +
        rows
          .slice(1)
          .map((row) => `| ${row} |`)
          .join("\n") +
        "\n\n"
      );
    }
    if (tag === "a") {
      const href = node.getAttribute("href");
      return href && !href.startsWith("#")
        ? `[${inner.trim()}](${new URL(href, origin + meta.path).href})`
        : inner;
    }
    if (tag === "li") {
      const marker =
        node.parentElement?.localName === "ol"
          ? `${Array.from(node.parentElement.children).indexOf(node) + 1}.`
          : "-";
      return `\n${marker} ${inner.trim().replace(/\n\n/g, "\n  ")}`;
    }
    if (tag === "pre")
      return `\n\n\`\`\`\n${node.textContent.trim()}\n\`\`\`\n\n`;
    if (tag === "br") return " ";
    return [
      "p",
      "div",
      "section",
      "ul",
      "ol",
      "table",
      "tr",
      "article",
    ].includes(tag)
      ? `\n\n${inner}\n\n`
      : inner;
  }
  const body = text(main)
    .replace(/\n{3,}/g, "\n\n")
    .trim();
  if (!body.includes("# ") || body.length < 40)
    throw new Error(`Empty machine body: ${meta.path}`);
  return `---\ntitle: ${JSON.stringify(meta.title)}\nlanguage: ${meta.locale}\ncanonical: ${origin}${meta.path}\ncontent_revision: ${revision}\n---\n\n${body}\n`;
}
