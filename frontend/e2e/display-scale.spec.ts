import { expect, test } from "@playwright/test";

const tokenClasses = [
  "text-9", "text-10", "text-11", "text-12", "text-13", "text-14", "text-15",
  "text-16", "text-17", "text-20", "text-22", "text-24", "text-28", "text-36",
];

const readingSizes = {
  9: 12, 10: 12, 11: 13, 12: 14, 13: 15, 14: 14, 15: 15,
  16: 16, 17: 17, 20: 20, 22: 22, 24: 24, 28: 28, 36: 36,
};

for (const theme of ["light", "dark"]) {
  for (const scale of [0.5, 1, 2]) {
    test(`Studio and assistant compute the same reading sizes (${theme}, ${scale}×)`, async ({ page }) => {
      // Load the real compiled stylesheet without mounting an app or calling APIs.
      await page.route("**/__display-scale-test", (route) => route.fulfill({
        contentType: "text/html",
        body: `<html class="theme-${theme}" style="font-size:${85 * scale}%"><head></head><body></body></html>`,
      }));
      await page.goto("/__display-scale-test");
      await page.addStyleTag({ url: "/src/app.css?direct" });
      const sizes = await page.evaluate((classes) => {
        function render(surface: string) {
          const section = document.createElement("section");
          section.setAttribute("aria-label", surface);
          for (const tokenClass of classes) {
            const label = document.createElement("span");
            label.className = tokenClass;
            label.textContent = `${surface} label`;
            section.append(label);
          }
          document.body.append(section);
          const roles = document.createElement("div");
          roles.innerHTML = `<a class="shell-nav-item"><svg class="lucide"></svg>Destination</a>
            <p class="shell-group-label">Workspace</p>
            <p class="shell-user-name">Reader</p><p class="shell-user-email">reader@example.test</p>`;
          section.append(roles);
          const roleSizes = [...roles.children].map((element) => {
            const style = getComputedStyle(element);
            return { size: style.fontSize, weight: style.fontWeight };
          });
          const icon = roles.querySelector("svg")!.getBoundingClientRect();
          return {
            tokens: [...section.querySelectorAll(":scope > span")].map((element) => Number.parseFloat(getComputedStyle(element).fontSize)),
            roles: roleSizes, icon: { width: icon.width, height: icon.height },
          };
        }
        const studio = render("Studio");
        const studioLeading = getComputedStyle(document.body).lineHeight;
        document.documentElement.classList.add("assistant-display");
        const assistant = render("Assistant");
        const assistantLeading = getComputedStyle(document.body).lineHeight;
        return { studio, assistant, studioLeading, assistantLeading };
      }, tokenClasses);
      expect(sizes.assistant).toEqual(sizes.studio);
      for (const [index, size] of [15, 12, 14, 12].entries()) {
        expect(Number.parseFloat(sizes.studio.roles[index].size)).toBeCloseTo(size * 0.85 * scale, 3);
      }
      expect(sizes.studio.roles.map((role) => role.weight)).toEqual(["400", "500", "500", "400"]);
      expect(sizes.studio.icon.width).toBeCloseTo(15 * 0.85 * scale * 1.125, 1);
      expect(sizes.assistantLeading).toBe(sizes.studioLeading);
      expect(Number.parseFloat(sizes.studioLeading)).toBeCloseTo(16 * 0.85 * scale * 1.5, 3);
      for (const [index, size] of Object.values(readingSizes).entries()) {
        expect(sizes.studio.tokens[index]).toBeCloseTo(size * 0.85 * scale, 3);
      }

      const unthemed = await page.evaluate(() => {
        const root = document.documentElement;
        root.className = "";
        root.style.fontSize = "";
        return [...document.querySelectorAll("section:first-child span")].map((element) => Number.parseFloat(getComputedStyle(element).fontSize));
      });
      expect(unthemed.slice(0, 5)).toEqual([9, 10, 11, 12, 13]);
    });
  }
}
