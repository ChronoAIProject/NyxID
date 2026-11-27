import fs from "node:fs/promises";
import path from "node:path";
import { build } from "vite";
import { Window } from "happy-dom";
import { renderPage, metadataHead, PUBLIC_ORIGIN } from "../.public-render/public-render.js";

const root = path.resolve(import.meta.dirname, "..");
const output = path.join(root, "dist-landing");
const linkOrigin = process.env.LANDING_LINK_ORIGIN || PUBLIC_ORIGIN;
const target = new URL(linkOrigin);
if (!["http:", "https:"].includes(target.protocol) || target.origin !== linkOrigin)
  throw new Error("LANDING_LINK_ORIGIN must be an HTTP(S) origin without a trailing slash");

let assets;
await build({
  configFile: false,
  root,
  publicDir: false,
  build: {
    outDir: output,
    emptyOutDir: true,
    rollupOptions: {
      input: path.join(root, "src/features/static-landing/index.ts"),
      output: { entryFileNames: "assets/landing-[hash].js" },
    },
  },
  plugins: [{
    name: "standalone-landing-contract",
    generateBundle: {
      order: "post",
      handler(_options, bundle) {
        for (const chunk of Object.values(bundle)) {
          if (chunk.type !== "chunk") continue;
          const forbidden = Object.keys(chunk.modules).filter(id => /node_modules\/(?:react(?:-dom)?\/|@tanstack\/)/.test(id));
          if (forbidden.length) this.error("Standalone landing browser output includes React or TanStack");
        }
        const entry = Object.values(bundle).find(chunk => chunk.type === "chunk" && chunk.isEntry);
        if (!entry) this.error("Missing standalone browser entry");
        assets = { script: entry.fileName, styles: Object.values(bundle).filter(asset => asset.type === "asset" && asset.fileName.endsWith(".css")).map(asset => asset.fileName) };
      },
    },
  }],
});
if (!assets?.styles.length) throw new Error("Missing standalone landing CSS");
const assetPaths = new Set();
for (const route of ["/", "/zh"]) {
  const { html, meta } = await renderPage(route, async () => { throw new Error("Homepage must not read docs"); });
  const document = new Window().document;
  document.body.innerHTML = html;
  const markup = document.querySelector(".nyx-home");
  if (!markup) throw new Error("Missing approved homepage");

  // Make destinations explicit even when JavaScript is disabled. The browser
  // script adds visitor query parameters without changing the HTML content.
  for (const anchor of markup.querySelectorAll('a[href^="/"]'))
    if (!["/", "/zh"].includes(new URL(anchor.getAttribute("href"), linkOrigin).pathname))
      anchor.href = new URL(anchor.getAttribute("href"), linkOrigin).href;

  for (const element of markup.querySelectorAll("img[src],canvas[data-nyx]")) {
    const value = element.getAttribute("src") || element.getAttribute("data-nyx");
    if (value?.startsWith("/") && !value.startsWith("//")) assetPaths.add(value);
  }
  const page = `<!doctype html>
  <html lang="${meta.locale === "zh" ? "zh-CN" : "en"}" data-landing-link-origin="${encodeURI(linkOrigin)}"><head>
  <meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="theme-color" content="#F3F3F0">
  ${metadataHead(meta, linkOrigin)}
  ${assets.styles.map(file => `<link rel="stylesheet" href="/${file}">`).join("\n")}
  <script type="module" src="/${assets.script}"></script>
  </head><body>${markup.outerHTML}</body></html>\n`;
  const pageOutput = path.join(output, route === "/" ? "index.html" : "zh/index.html");
  await fs.mkdir(path.dirname(pageOutput), { recursive: true });
  await fs.writeFile(pageOutput, page);
}
for (const stylesheet of assets.styles) {
  const css = await fs.readFile(path.join(output, stylesheet), "utf8");
  for (const match of css.matchAll(/url\(["']?(\/[^)"']+)["']?\)/g))
    if (!match[1].startsWith("//")) assetPaths.add(match[1]);
}
for (const asset of assetPaths) {
  const source = path.resolve(root, "public", asset.slice(1));
  if (!source.startsWith(path.join(root, "public") + path.sep)) throw new Error("Invalid homepage asset path");
  const destination = path.join(output, asset.slice(1));
  await fs.mkdir(path.dirname(destination), { recursive: true });
  await fs.copyFile(source, destination);
}
await fs.writeFile(path.join(output, "robots.txt"), "User-agent: *\nDisallow: /\n");
console.log(`Standalone HTML homepages: ${output}/index.html and ${output}/zh/index.html`);
console.log("Browser graph verified: no React or TanStack runtime.");
