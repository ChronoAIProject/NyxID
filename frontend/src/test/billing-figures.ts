/** Collect numeric text with its rendered label, preserving duplicate occurrences.
 * No money is recalculated here. The same collector runs against the base DOM.
 */
export function billingFigures(root: HTMLElement): string[] {
  const blocks = new Set<Element>();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  while (walker.nextNode()) {
    const node = walker.currentNode;
    if (!/\d|≥|^Unavailable$|^—$/.test(node.textContent ?? "")) continue;
    const parent = node.parentElement;
    if (!parent || parent.closest('[hidden],[inert],[aria-hidden="true"],button,[role="combobox"],input,select,script,style,[data-usage-summary="compact"]')) continue;
    let block = parent;
    while (block.parentElement && /^(SPAN|SMALL|B|EM)$/.test(block.tagName)) block = block.parentElement;
    blocks.add(block);
  }
  const text = (el: Element | null) => el?.textContent?.replace(/\s+/g, " ").trim() ?? "";
  return [...blocks].map(block => {
    let label = "";
    if (block.tagName === "DD") label = text(block.previousElementSibling);
    else if (block.tagName === "TD") {
      const table = block.closest("table")!;
      label = `${text(table.querySelectorAll("th")[Array.from(block.parentElement!.children).indexOf(block)] ?? null)} / ${text(block.parentElement!.firstElementChild)}`;
    } else if (block.tagName === "P" && !block.hasAttribute("title")) label = block.closest("li") ? text(block.closest("li")!.querySelector("strong")) : "";
    else if (block.tagName === "STRONG" && block.closest(".summary-family")) label = text(block.parentElement!.querySelector("h3"));
    else { const previous = text(block.previousElementSibling); label = previous.length <= 140 ? previous : ""; }
    return `${label} | ${text(block)}`;
  }).sort();
}
