import { cloneElement, isValidElement } from "react";
import { renderRules, type RenderRules } from "react-native-markdown-display";

// These rules return the top-level native Text for paragraphs/headings/table
// cells or code blocks. Preserve the package's styling and newline handling.
export const assistantSelectableMarkdown: RenderRules = Object.fromEntries(
  ["textgroup", "code_block", "fence"].map((type) => [
    type,
    (...args: Parameters<NonNullable<RenderRules[string]>>) => {
      const element = renderRules[type]?.(...args);
      return isValidElement<{ selectable?: boolean }>(element)
        ? cloneElement(element, { selectable: true })
        : element;
    },
  ]),
);
