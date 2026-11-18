import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, it } from "vitest";
import {
  DEFAULT_PUBLIC_ORIGIN,
  buildLegalMarkdown,
  buildRobots,
  buildSitemap,
  publicUrl,
} from "../scripts/public-origin.mjs";

const env = (origin: string) => ({ NYXID_PUBLIC_ORIGIN: origin });

it("uses the canonical origin for sitemap, robots and legal output by default", () => {
  const robots = readFileSync(resolve("public/robots.txt"), "utf8");
  const terms = readFileSync(resolve("public/legal/terms.md"), "utf8");

  expect(DEFAULT_PUBLIC_ORIGIN).toBe("https://nyxid.ai");
  expect(buildSitemap(["/", "/docs/start"])).toContain(
    "<loc>https://nyxid.ai/docs/start</loc>",
  );
  expect(buildRobots(robots)).toContain("Sitemap: https://nyxid.ai/sitemap.xml");
  expect(publicUrl("/terms")).toBe("https://nyxid.ai/terms");
  expect(buildLegalMarkdown(terms)).toContain("Website: **https://nyxid.ai/**");
});

it("applies one configured build-time origin to all public output", () => {
  const custom = env("https://preview.example.test/");
  const robots = readFileSync(resolve("public/robots.txt"), "utf8");
  const terms = readFileSync(resolve("public/legal/terms.md"), "utf8");

  expect(buildSitemap(["/", "/privacy"], custom)).toContain(
    "<loc>https://preview.example.test/privacy</loc>",
  );
  expect(buildRobots(robots, custom)).toContain(
    "Sitemap: https://preview.example.test/sitemap.xml",
  );
  expect(publicUrl("/privacy", custom)).toBe("https://preview.example.test/privacy");
  expect(buildLegalMarkdown(terms, custom)).toContain(
    "Website: **https://preview.example.test/**",
  );
});

