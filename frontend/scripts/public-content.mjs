import { Window } from "happy-dom";
export function semanticMarkdown(html, meta, revision, origin) {
  const window = new Window();
  const doc = window.document;
  doc.body.innerHTML = html;
  const main = doc.querySelector("main");
  if (!main) throw new Error(`Missing semantic main: ${meta.path}`);
  // Only explicitly approved reusable panels are expanded for machine reading.
  main
    .querySelectorAll(
      'div[role="tabpanel"][data-public-panel="audience"],pre[data-public-panel="code"]',
    )
    .forEach((panel) => {
      panel.removeAttribute("hidden");
      const label =
        panel.getAttribute("data-public-label") ??
        doc
          .getElementById(panel.getAttribute("aria-labelledby"))
          ?.textContent?.trim();
      if (label) {
        const heading = doc.createElement("h3");
        heading.textContent = label;
        panel.before(heading);
      }
    });
  const escape = (value) => value.replace(/([\\`*_[\]<>])/g, "\\$1");
  main
    .querySelectorAll(
      'script,style,svg,[aria-hidden="true"],[hidden],button,input,select,textarea',
    )
    .forEach((el) => el.remove());
  function text(node) {
    if (node.nodeType === 3)
      return escape(node.textContent.replace(/\s+/g, " "));
    if (node.nodeType !== 1) return "";
    const inner = Array.from(node.childNodes, text).join(""),
      tag = node.localName;
    if (/^h[1-6]$/.test(tag))
      return `\n\n${"#".repeat(Number(tag[1]))} ${inner}\n\n`;
    if (tag === "img")
      return node.getAttribute("alt")
        ? `![${node.getAttribute("alt")}](${new URL(node.getAttribute("src"), origin + meta.path).href})`
        : "";
    if (tag === "code" && node.parentElement?.localName !== "pre") {
      const value = node.textContent.replace(/\s+/g, " ");
      const fence = "`".repeat(
        Math.max(0, ...(value.match(/`+/g) ?? []).map((run) => run.length)) + 1,
      );
      return `${fence} ${value} ${fence}`;
    }
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
      if (!href || href.startsWith("#")) return inner;
      const destination = new URL(href, origin + meta.path).href;
      const link = (label) => `[${label}](<${destination}>)`;
      if (node.querySelector("h1,h2,h3,h4,h5,h6,p,div,section")) {
        const lead = node.querySelector("h1,h2,h3,h4,h5,h6,b,strong,p");
        if (lead) {
          const original = text(lead);
          const title = Array.from(lead.childNodes, text).join("").trim();
          const replacement = /^h[1-6]$/.test(lead.localName)
            ? `\n\n${"#".repeat(Number(lead.localName[1]))} ${link(title)}\n\n`
            : `\n\n${link(title)}\n\n`;
          return `\n\n${inner.replace(original, replacement)}\n\n`;
        }
        return `\n\n${link(escape(node.textContent.replace(/\s+/g, " ").trim()))}\n\n`;
      }
      return link(inner.trim());
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
