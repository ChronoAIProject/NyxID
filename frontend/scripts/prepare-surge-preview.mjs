import fs from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { getPublicOrigin } from "./public-origin.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(root, "dist");
const output = path.join(root, ".surge-preview");
const domain = process.argv[2] || "ctkm-nyx-landing.surge.sh";
if (!/^[a-z0-9][a-z0-9-]*\.surge\.sh$/.test(domain)) throw new Error("Expected a surge.sh preview domain");
const release = JSON.parse(await fs.readFile(path.join(dist, "public-release.json"), "utf8"));
const validation = JSON.parse(await fs.readFile(path.join(dist, "public-validation.json"), "utf8"));
if (validation.status !== "pass") throw new Error("Build must pass public validation before packaging");
const appRoutes = JSON.parse(await fs.readFile(path.join(dist, "application-routes.json"), "utf8"));
const liveOrigin = new URL(getPublicOrigin()).origin;
if (liveOrigin === `https://${domain}`) throw new Error("Live account handoff must point to the production origin");
const appPatterns = appRoutes.map(route => "^" + route.split("/").map(segment =>
  segment.startsWith("$") ? (segment === "$" ? ".+" : "[^/]+") : segment.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
).join("/") + "/?$");

await fs.rm(output, { recursive: true, force: true });
await fs.mkdir(output, { recursive: true });
await fs.cp(dist, output, {
  recursive: true,
  filter: (source) => {
    const relative = path.relative(dist, source);
    if (!relative) return true;
    return !relative.split(path.sep).some((segment) => ["__public", "credential-accept", "node_modules"].includes(segment)) &&
      !/\.(?:conf|map)$/.test(relative) &&
      !["app.html", "index.html", "application-routes.json", "public-release.json", "public-validation.json"].includes(relative);
  },
});

const noindex = (html) => html.replace(/<meta\b[^>]*name=["']robots["'][^>]*>/gi, "")
  .replace("</head>", '<meta name="robots" content="noindex,nofollow"></head>');
async function writeRoute(route, html) {
  if (route === "/") return fs.writeFile(path.join(output, "index.html"), noindex(html));
  const relative = route.replace(/^\//, "");
  if (!/^[a-z0-9/-]+$/.test(relative) || relative.includes("..")) throw new Error(`Invalid route: ${route}`);
  await fs.mkdir(path.dirname(path.join(output, `${relative}.html`)), { recursive: true });
  await fs.writeFile(path.join(output, `${relative}.html`), noindex(html));
  await fs.mkdir(path.join(output, relative), { recursive: true });
  await fs.writeFile(path.join(output, relative, "index.html"), noindex(html));
}

for (const page of release.pages) {
  await writeRoute(page.path, await fs.readFile(path.join(dist, page.file), "utf8"));
}

// Directory hosting may append a slash. Normalize it before the public router hydrates.
const previewScript = `(()=>{const live=${JSON.stringify(liveOrigin)};const app=${JSON.stringify(appPatterns)}.map(p=>new RegExp(p));const isApp=p=>app.some(pattern=>pattern.test(p));const route=location.pathname.replace(/\\/+$/,"")||"/";if(isApp(route)){location.replace(live+route+location.search+location.hash);return}if(route!==location.pathname)history.replaceState(history.state,"",route+location.search+location.hash);document.addEventListener("click",e=>{const a=e.target?.closest?.("a[href]");if(!a)return;const url=new URL(a.href,location.href);if(url.origin===location.origin&&isApp(url.pathname)){e.preventDefault();location.assign(live+url.pathname+url.search+url.hash)}},true)})();`;
await fs.writeFile(path.join(output, "static-preview.js"), previewScript);

const fallback = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Page not found · NyxID</title><meta name="robots" content="noindex,nofollow"><script src="/static-preview.js"></script><style>body{margin:0;background:#F3F3F0;color:#17171A;font:18px/1.5 system-ui,sans-serif}main{max-width:600px;margin:16vh auto;padding:24px}img{height:28px}h1{font-size:44px;font-weight:500;letter-spacing:-.04em}a{color:inherit;text-underline-offset:4px}</style></head><body><main><a href="/"><img src="/nyxid-coloured-logo-dark.svg" alt="NyxID"></a><h1>Page not found.</h1><p><a href="/">Back to NyxID</a></p></main></body></html>`;
await fs.writeFile(path.join(output, "200.html"), fallback);
await fs.writeFile(path.join(output, "404.html"), fallback);
for (const route of appRoutes.filter(route=>!route.includes("$"))) await writeRoute(route, fallback);

// An external script keeps normal navigation, locale query and history working on Surge.
async function inject(directory) {
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) await inject(target);
    else if (entry.name.endsWith(".html")) {
      const html = await fs.readFile(target, "utf8");
      if (!html.includes('src="/static-preview.js"')) await fs.writeFile(target, html.replace("</head>", '<script src="/static-preview.js"></script></head>'));
    }
  }
}
await inject(output);
await fs.writeFile(path.join(output, "robots.txt"), "User-agent: *\nDisallow: /\n");
await fs.writeFile(path.join(output, "CNAME"), `${domain}\n`);
console.log(`Prepared ${release.pages.length} public routes for https://${domain}`);
console.log(`Upload directory: ${output}`);
console.log(`Account links hand off to ${liveOrigin}`);
