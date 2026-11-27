import { authHref, injectRedditDefaultCode, legacyDestination, siteHref } from "../website-a/auth-links";
import { catalogEntries, matchesIntegration } from "../website-a/nyx-home/catalog-data";
import { homeText } from "../website-a/nyx-home/home-copy";
import "../website-a/public-design.css";
import "../website-a/nyx-home/nyx-home.css";
import "./standalone.css";

const page = document.querySelector<HTMLElement>(".nyx-home");
const configuredOrigin = document.documentElement.dataset.landingLinkOrigin;
if (!page || !configuredOrigin) throw new Error("Missing standalone landing configuration");
const language = page.lang === "zh-CN" ? "zh" : "en";
const landingText = (english: Parameters<typeof homeText>[0]) => homeText(english, language);
// Directory-only servers append a slash; keep the published locale URL canonical.
if (location.pathname === "/zh/") history.replaceState(history.state, "", "/zh" + location.search + location.hash);
const destination = new URL(configuredOrigin);
if (!["http:", "https:"].includes(destination.protocol) || destination.origin !== configuredOrigin)
  throw new Error("Invalid standalone landing origin");
const linkOrigin = destination.origin;
const controller = new AbortController();
const { signal } = controller;

injectRedditDefaultCode();
const legacy = legacyDestination(location.hash);
if (legacy) location.replace(new URL(legacy, linkOrigin));

// Keep the authored links and visitor query intact, including repeated values.
for (const anchor of page.querySelectorAll<HTMLAnchorElement>("a[href]")) {
  const href = anchor.getAttribute("href")!;
  if (href.startsWith("#")) continue;
  const url = new URL(href, linkOrigin);
  if (url.origin !== linkOrigin) continue;
  const path = url.pathname + url.search + url.hash;
  const target = /^\/(?:login|register)(?:\?|$)/.test(path)
    ? authHref(path.startsWith("/login") ? "login" : "register")
    : siteHref(path);
  anchor.href = ["/", "/zh"].includes(url.pathname) ? target : new URL(target, linkOrigin).href;
}

let preferredTheme: "light" | "dark" | null = null;
try {
  const value = localStorage.getItem("nyxid-theme");
  if (value === "light" || value === "dark") preferredTheme = value;
} catch { /* Browser storage is optional. */ }
const systemTheme = matchMedia("(prefers-color-scheme: dark)");
function applyTheme() {
  const theme = preferredTheme ?? (systemTheme.matches ? "dark" : "light");
  page!.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", theme === "dark" ? "#0E0E10" : "#F3F3F0");
  page!.querySelector(".public-theme-toggle")?.setAttribute("aria-label", language === "zh" ? (theme === "dark" ? "切换日间模式" : "切换夜间模式") : (theme === "dark" ? "Switch to day mode" : "Switch to night mode"));
}
applyTheme();
systemTheme.addEventListener("change", () => { if (!preferredTheme) applyTheme(); }, { signal });
window.addEventListener("storage", event => {
  if (event.key !== "nyxid-theme" && event.key !== null) return;
  const value = event.newValue;
  preferredTheme = value === "light" || value === "dark" ? value : null;
  applyTheme();
}, { signal });
page.querySelector(".public-theme-toggle")?.addEventListener("click", () => {
  const update = () => {
    preferredTheme = page.dataset.theme === "dark" ? "light" : "dark";
    try { localStorage.setItem("nyxid-theme", preferredTheme); } catch { /* Keep this visit's choice. */ }
    applyTheme();
  };
  if (document.startViewTransition && !matchMedia("(prefers-reduced-motion: reduce)").matches)
    document.startViewTransition(update);
  else update();
}, { signal });

for (const menu of page.querySelectorAll<HTMLDetailsElement>(".public-more, .public-language")) {
  menu.addEventListener("keydown", event => {
    if (event.key !== "Escape") return;
    menu.open = false;
    menu.querySelector("summary")?.focus();
  }, { signal });
  menu.addEventListener("focusout", event => {
    if (!menu.contains(event.relatedTarget as Node | null)) menu.open = false;
  }, { signal });
  menu.addEventListener("click", event => {
    if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    if ((event.target as Element).closest("a")) menu.open = false;
  }, { signal });
}

const catalog = page.querySelector<HTMLDetailsElement>(".tool-catalog");
if (catalog) {
  const input = catalog.querySelector<HTMLInputElement>('input[type="search"]')!;
  const status = catalog.querySelector<HTMLElement>('[role="status"]')!;
  const empty = document.createElement("p");
  empty.className = "catalog-empty";
  empty.textContent = landingText("No tools found. Try another name.");
  empty.hidden = true;
  catalog.querySelector(".catalog-head")!.after(empty);
  const tiles = [...catalog.querySelectorAll<HTMLElement>(".catalog-tile")].map(element => ({
    element,
    entry: catalogEntries.find(tool => tool.label === element.querySelector(".catalog-tool-copy > span")?.textContent),
  }));
  input.addEventListener("input", () => {
    const search = input.value.trim().toLowerCase();
    let count = 0;
    for (const { element, entry } of tiles) {
      element.hidden = entry ? !matchesIntegration(entry, search) : !element.textContent?.toLowerCase().includes(search);
      if (!element.hidden) count++;
    }
    for (const group of catalog.querySelectorAll<HTMLElement>(".catalog-group")) {
      const visible = group.querySelectorAll(".catalog-tile:not([hidden])").length;
      group.hidden = visible === 0;
      group.querySelector("h3 span")!.textContent = String(visible);
    }
    status.textContent = search ? (language === "zh" ? `找到 ${count} 个工具` : `${count} ${count === 1 ? "tool" : "tools"} found`) : landingText("Connect with your own accounts or credentials.");
    empty.hidden = count !== 0;
  }, { signal });
  catalog.querySelector(".catalog-footer button")?.addEventListener("click", () => {
    catalog.open = false;
    catalog.querySelector("summary")?.focus();
  }, { signal });
}
for (const image of page.querySelectorAll<HTMLImageElement>("img"))
  image.addEventListener("error", () => { image.style.visibility = "hidden"; }, { signal });

// Reuse the approved motion implementation; React does not run in this page.
let stopMotion: (() => void) | undefined;
void import("../website-a/nyx-home/nyx-home-motion").then(({ startNyxHome }) => {
  if (signal.aborted) return;
  page.classList.add("js");
  try { stopMotion = startNyxHome(page); }
  catch (error) { page.classList.remove("js"); console.error("Landing motion unavailable", error); }
}).catch(error => console.error("Landing motion unavailable", error));

// A static preview needs no session to render. Read only to preserve the home
// redirect when the page is hosted alongside the application's cookie session.
void fetch(new URL("/api/v1/users/me", linkOrigin), {
  credentials: "include", headers: { "Content-Type": "application/json" },
  signal: AbortSignal.any([signal, AbortSignal.timeout(5000)]),
}).then(async response => {
  if (!response.ok) return;
  const user: unknown = await response.json();
  if (user && typeof user === "object" && "id" in user && typeof user.id === "string" && user.id)
    location.replace(new URL("/assistant", linkOrigin));
}).catch(() => { /* Unavailable APIs leave the static homepage usable. */ });

document.documentElement.dataset.staticReady = "v1";
window.addEventListener("pagehide", event => {
  if (event.persisted) return;
  stopMotion?.();
  controller.abort();
}, { once: true });
