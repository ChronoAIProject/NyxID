import { expect, type Locator } from "@playwright/test";

export async function expectInternalContainment(containers: Locator) {
  for (const container of await containers.all()) {
    const metrics = await container.evaluate(element => ({
      name: element.getAttribute("aria-label") ?? element.getAttribute("role") ?? element.tagName,
      width: element.clientWidth,
      scroll: element.scrollWidth,
    }));
    expect(metrics.scroll, JSON.stringify(metrics)).toBeLessThanOrEqual(metrics.width + 1);
  }
}

/** Button boxes alone do not catch a nowrap label escaping a shrunken flex item. */
export async function expectActionTextContained(footers: Locator) {
  await expectInternalContainment(footers);
  for (const button of await footers.locator("button:visible").all()) {
    const metrics = await button.evaluate(button => {
      const outer = button.getBoundingClientRect(), style = getComputedStyle(button);
      const walker = document.createTreeWalker(button, NodeFilter.SHOW_TEXT);
      const bounds = [];
      while (walker.nextNode()) {
        const text = walker.currentNode;
        if (!text.textContent?.trim() || text.parentElement?.closest(".sr-only")) continue;
        const range = document.createRange();
        range.selectNodeContents(text);
        for (const rect of range.getClientRects()) bounds.push({
          left: rect.left - outer.left - parseFloat(style.paddingLeft),
          right: outer.right - rect.right - parseFloat(style.paddingRight),
          top: rect.top - outer.top,
          bottom: outer.bottom - rect.bottom,
        });
      }
      return { name: button.textContent, width: button.clientWidth, scroll: button.scrollWidth, bounds };
    });
    expect(metrics.scroll, JSON.stringify(metrics)).toBeLessThanOrEqual(metrics.width + 1);
    for (const rect of metrics.bounds) for (const value of Object.values(rect)) expect(value, JSON.stringify(metrics)).toBeGreaterThanOrEqual(-1);
  }
}
