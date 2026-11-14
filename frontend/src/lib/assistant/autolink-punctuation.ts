import type { Root, RootContent } from "mdast";

import {
  bareAutolinkBoundary,
  splitBareAutolinks,
} from "../../../../shared/autolinkPunctuation";

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
        if (!/^https?:\/\//i.test(raw)) return [node];
        const boundary = bareAutolinkBoundary(raw);
        if (boundary === raw.length) return [node];
        return splitBareAutolinks(raw).map(
          (part): RootContent =>
            part.href
              ? {
                  type: "link",
                  url: part.href,
                  children: [{ type: "text", value: part.text }],
                }
              : { type: "text", value: part.text },
        );
      });
    }
    visit(tree);
  };
}
