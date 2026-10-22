import { expect, it } from "vitest";
import { assertPublicConfiguration } from "./configuration-boundary";
it("rejects same-switch HTML/JS with different effective endpoints before hydration", () => {
  const root = document.createElement("div");
  root.dataset.publicRender = "v1";
  const first = JSON.stringify({ newsletter: { endpoint: "/api/first" } }),
    second = JSON.stringify({ newsletter: { endpoint: "/api/second" } });
  root.setAttribute("data-website-config", encodeURIComponent(first));
  expect(() => assertPublicConfiguration(root, first)).not.toThrow();
  expect(() => assertPublicConfiguration(root, second)).toThrow(/mismatch/);
  root.removeAttribute("data-website-config");
  expect(() => assertPublicConfiguration(root, first)).toThrow(/mismatch/);
});
