import { expect, type Page } from "@playwright/test";

/** Measure actual painted toast colors, including transparent child backgrounds. */
export async function expectReadableToast(page: Page) {
  const toast = page.locator("[data-sonner-toast]").filter({ visible: true }).last();
  await expect(toast.locator("[data-description]")).toBeVisible();
  const contrast = await toast.evaluate(toast => {
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = 1;
    const ctx = canvas.getContext("2d")!;
    const rgba = (color: string) => {
      ctx.clearRect(0, 0, 1, 1);
      ctx.fillStyle = color;
      ctx.fillRect(0, 0, 1, 1);
      return [...ctx.getImageData(0, 0, 1, 1).data].map((value, i) => i === 3 ? value / 255 : value);
    };
    const over = (front: number[], back: number[]) => front.slice(0, 3).map((value, i) => value * front[3]! + back[i]! * (1 - front[3]!));
    const background = (element: Element | null): number[] => {
      if (!element) return [255, 255, 255];
      const color = rgba(getComputedStyle(element).backgroundColor);
      return color[3] === 1 ? color.slice(0, 3) : over(color, background(element.parentElement));
    };
    const luminance = (color: number[]) => color.map(value => {
      const c = value / 255;
      return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
    }).reduce((sum, value, i) => sum + value * [0.2126, 0.7152, 0.0722][i]!, 0);
    return [...toast.querySelectorAll("[data-title], [data-description], [data-button]")].map(element => {
      const bg = background(element);
      const fg = over(rgba(getComputedStyle(element).color), bg);
      const a = luminance(fg), b = luminance(bg);
      return { text: element.textContent, foreground: fg, background: bg, ratio: (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05) };
    });
  });
  expect(contrast.length).toBeGreaterThanOrEqual(4);
  for (const reading of contrast) expect(reading.ratio, JSON.stringify(reading)).toBeGreaterThanOrEqual(4.5);
  return contrast;
}
