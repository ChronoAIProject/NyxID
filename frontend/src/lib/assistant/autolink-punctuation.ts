import type { Root, RootContent } from "mdast";
import { unified } from "unified";
import remarkParse from "remark-parse";
import remarkGfm from "remark-gfm";
const literalParser = unified().use(remarkParse).use(remarkGfm);

import { splitBareAutolinks } from "../../../../shared/autolinkPunctuation";

/** GFM literals only: preserve explicit Markdown and angle-bracket links. */
export function remarkAutolinkPunctuation() {
  return (tree: Root, file: { value: unknown }) => {
    const source = String(file.value);
    function visit(parent: { children?: RootContent[] }) {
      if (!parent.children) return;
      parent.children = parent.children.flatMap((node): RootContent[] => {
        if (node.type !== "link") {
          if ("children" in node) visit(node as { children: RootContent[] });
          return [node];
        }
        const start = node.position?.start.offset;
        const end = node.position?.end.offset;
        if (start === undefined || end === undefined) return [node];
        const raw = source.slice(start, end);
        if (/^[<[]/.test(raw)) return [node];
        if (!/[^\p{ASCII}]/u.test(raw)) return [node];
        return splitBareAutolinks(raw).flatMap((part): RootContent[] => {
          if (!part.href) return [{ type: "text", value: part.text }];
          const parsed = literalParser.parse(part.text).children[0];
          return parsed?.type === "paragraph"
            ? parsed.children
            : [{ type: "text", value: part.text }];
        });
      });
    }
    visit(tree);
  };
}
