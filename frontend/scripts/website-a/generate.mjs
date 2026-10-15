// One-time, deterministic migration of the approved A HTML into React JSX.
// The source is a design input, never loaded or executed by the production app.
import fs from "node:fs";
import vm from "node:vm";
import crypto from "node:crypto";
import path from "node:path";
import { Window } from "happy-dom";
import postcss from "postcss";
import { format } from "prettier";

const root = path.resolve(import.meta.dirname, "../..");
const source = fs.readFileSync(
  process.argv[2] ??
    path.resolve(root, "../docs/plans/website-review/nyxid-site-preview.html"),
  "utf8",
);
const hash = crypto.createHash("sha256").update(source).digest("hex");
if (hash !== "5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33")
  throw new Error("Unexpected A baseline");
const script = source.slice(
  source.lastIndexOf("<script>") + 8,
  source.lastIndexOf("</script>"),
);
const declarations = script
  .slice(0, script.indexOf("/* ═════════ router"))
  .replace("const tryInner =", "let tryInner =");
const dom = new Window();
const assetDir = path.join(root, "public/website-a");
const outDir = path.join(root, "src/features/website-a/generated");
fs.mkdirSync(assetDir, { recursive: true });
fs.mkdirSync(outDir, { recursive: true });
const q = JSON.stringify;
const attrNames = {
  class: "className",
  for: "htmlFor",
  tabindex: "tabIndex",
  "stroke-width": "strokeWidth",
  "stroke-linecap": "strokeLinecap",
  "stroke-linejoin": "strokeLinejoin",
  "stroke-dasharray": "strokeDasharray",
  "stroke-dashoffset": "strokeDashoffset",
  "fill-rule": "fillRule",
  "clip-rule": "clipRule",
  "fill-opacity": "fillOpacity",
  "stroke-opacity": "strokeOpacity",
  autocomplete: "autoComplete",
  readonly: "readOnly",
  colspan: "colSpan",
  srcset: "srcSet",
};
const boolAttrs = new Set([
  "hidden",
  "disabled",
  "required",
  "readOnly",
  "multiple",
  "open",
]);
const voids = new Set([
  "img",
  "input",
  "br",
  "hr",
  "meta",
  "link",
  "source",
  "wbr",
  "area",
  "embed",
  "col",
  "track",
]);
const camel = (k) =>
  k.startsWith("--") ? k : k.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
function style(el) {
  const result = {};
  const rule = postcss.parse(
    "x{" + (el.getAttribute("style") ?? "") + "}",
  ).first;
  rule.walkDecls((decl) => {
    result[camel(decl.prop)] = decl.value;
  });
  return result;
}
let locale;
function html(str) {
  const el = dom.document.createElement("div");
  el.innerHTML = str;
  return el;
}
function jsxChildren(el) {
  return [...el.childNodes].map(jsx).join("");
}
function jsx(el) {
  if (el.nodeType === 3) return el.textContent ? `{${q(el.textContent)}}` : "";
  if (el.nodeType !== 1) return "";
  const tag = el.localName,
    id = el.id,
    cls = el.getAttribute("class") ?? "";
  if (el.hasAttribute("data-skip")) return "";
  if (
    cls.split(" ").includes("ann") ||
    ["brand-guide", "design-guide", "site-map"].some(
      (s) => el.getAttribute("href") === `#/${s}`,
    )
  )
    return "";
  if (cls === "icp-tabs") {
    const tabs = [...el.querySelectorAll("[data-icp]")];
    const panels = tabs.map((t) =>
      el.parentElement.querySelector(`#icp-p-${t.dataset.icp}`),
    );
    panels.forEach((panel) => {
      panel
        .querySelectorAll(".checks li")
        .forEach((el, i) => el.style.setProperty("--i", String(i)));
      panel.querySelectorAll(".msgs > *").forEach((el, i) => {
        el.style.setProperty("--i", String(i + 1));
        el.style.setProperty("--step", "900ms");
      });
    });
    const entries = tabs.map((t, i) => {
      panels[i].setAttribute("data-skip", "");
      return `{id:${q(t.dataset.icp)},label:<>${jsxChildren(t)}</>,content:<>${jsxChildren(panels[i])}</>}`;
    });
    return `<AudienceSwitcher items={[${entries.join(",")}]} />`;
  }
  if (cls === "code") {
    return `<CodeExample labels={${q([...el.querySelectorAll(".tabs button")].map((b) => b.textContent))}} snippets={[${[...el.querySelectorAll("pre")].map((e) => `<>${jsxChildren(e)}</>`).join(",")}]} />`;
  }
  if (cls === "film") return "<SolutionFilm />";
  if (id === "nbSide") return "<DemoAgents />";
  if (id === "nbMsgs") return "<DemoMessages />";
  if (id === "nbChips") return "<DemoChips />";
  if (id === "intCount")
    return `<p className="small" role="status">{s.integrationCountLabel}</p>`;
  if (el.hasAttribute("data-gcount"))
    return `<span className="small">{s.integrationGroupCount(${q(el.closest("[data-group]").dataset.group)})}</span>`;
  let attrs = [];
  for (const a of el.attributes) {
    let name = attrNames[a.name] ?? a.name,
      value = a.value;
    if (
      /^on/i.test(name) ||
      name === "srcdoc" ||
      name === "selected" ||
      name === "data-skip"
    )
      continue;
    if (name === "style") {
      attrs.push(
        `style={${q(style(el))}${Object.keys(style(el)).some((k) => k.startsWith("--")) ? " as React.CSSProperties" : ""}}`,
      );
      continue;
    }
    if (name === "src" && value.startsWith("data:image/")) {
      const [prefix, bytes] = value.split(",");
      const ext = prefix.includes("svg") ? "svg" : "png";
      const filename =
        crypto.createHash("sha256").update(bytes).digest("hex").slice(0, 16) +
        "." +
        ext;
      fs.writeFileSync(
        path.join(assetDir, filename),
        Buffer.from(bytes, "base64"),
      );
      value = "/website-a/" + filename;
    }
    if (name === "href") {
      if (
        value === "#/login" ||
        value === "#/signup" ||
        value.startsWith("#/start") ||
        value === "#/done"
      ) {
        attrs.push(
          `href={s.authHref(${q(value === "#/login" ? "login" : "register")})}`,
        );
        continue;
      }
      if (value.startsWith("#/")) value = value.slice(1);
      if (value.startsWith("/") && !value.startsWith("//")) {
        attrs.push(`href={s.siteHref(${q(value)})}`);
        continue;
      }
    }
    if (
      name === "className" &&
      el.hasAttribute("data-v") &&
      (el.parentElement?.id === "catF" ||
        el.parentElement?.id === "chF" ||
        el.parentElement?.id === "blogF" ||
        el.parentElement?.id === "intAv")
    ) {
      const group = el.parentElement.id;
      attrs.push(`className={s.filterClass(${q(group)},${q(el.dataset.v)})}`);
      continue;
    }
    if (name === "data-nav")
      attrs.push(`aria-current={s.route===${q("/" + value)}?'page':undefined}`);
    if (name === "className" && el.hasAttribute("data-lang")) {
      attrs.push(
        `className={s.language===${q(value === "on" ? (locale === "en" ? "en" : "zh") : el.dataset.lang)}?'on':''}`,
      );
      continue;
    }
    if (name === "hidden" && cls.includes("int-group")) continue;
    if (boolAttrs.has(name)) attrs.push(`${name}={true}`);
    else if (name === "tabIndex") attrs.push(`tabIndex={${Number(value)}}`);
    else attrs.push(`${name}=${q(value)}`);
  }
  if (
    tag === "a" &&
    el.getAttribute("target") === "_blank" &&
    !el.hasAttribute("rel")
  )
    attrs.push('rel="noopener noreferrer"');
  if (tag === "img") {
    attrs.push("onError={s.imageFallback}");
  }
  if (tag === "button" && !el.hasAttribute("type")) attrs.push('type="button"');
  if (el.hasAttribute("data-lang"))
    attrs.push(`onClick={()=>s.setLanguage(${q(el.dataset.lang)})}`);
  if (cls === "nb-app")
    attrs.push("onMouseEnter={s.stopDemo} onFocusCapture={s.stopDemo}");
  if (id === "burger")
    attrs.push(
      'onClick={s.toggleMenu} aria-expanded={s.menuOpen} aria-controls="website-nav-links"',
    );
  if (cls === "links") attrs.push('id="website-nav-links"');
  if (
    el.hasAttribute("data-v") &&
    ["catF", "chF", "blogF", "intAv"].includes(el.parentElement?.id)
  )
    attrs.push(
      `onClick={()=>s.setFilter(${q(el.parentElement.id)},${q(el.dataset.v)})} aria-pressed={s.isFilterSelected(${q(el.parentElement.id)},${q(el.dataset.v)})}`,
    );
  if (el.parentElement?.id === "solGrid" && el.hasAttribute("data-category"))
    attrs.push(
      `hidden={!s.solutionVisible(${q(el.dataset.category)},${q(el.dataset.channels)})}`,
    );
  if (el.parentElement?.id === "blogGrid")
    attrs.push(`hidden={!s.postVisible(${q(el.dataset.category)})}`);
  if (id === "intQ")
    attrs.push("value={s.query} onChange={e=>s.setQuery(e.target.value)}");
  if (id === "intCat")
    attrs.push(
      "value={s.category} onChange={e=>s.setCategory(e.target.value)}",
    );
  if (cls.split(" ").includes("int") && el.hasAttribute("data-name"))
    attrs.push(
      `hidden={!s.integrationVisible(${q(el.dataset.name)},${q(el.dataset.cat)},${q(el.dataset.av)})}`,
    );
  if (el.hasAttribute("data-group"))
    attrs.push(`hidden={s.integrationGroupCount(${q(el.dataset.group)})===0}`);
  if (voids.has(tag)) return `<${tag} ${attrs.join(" ")} />`;
  return `<${tag} ${attrs.join(" ")}>${jsxChildren(el)}</${tag}>`;
}
const routes = [
  "",
  "nyxbot",
  "solutions",
  "how-it-works",
  "security",
  "agents",
  "pricing",
  "integrations",
  "blog",
  "contact",
  "notfound",
];
for (locale of ["en", "zh"]) {
  const stubs = { nav: { innerHTML: "" }, foot: { innerHTML: "" } };
  const context = vm.createContext({
    URLSearchParams,
    location: { search: `?lang=${locale}` },
    localStorage: { getItem: () => null },
    navigator: { language: locale },
    window: { NYX_TRY: {} },
    __ucDoc: () => "",
    document: { getElementById: (id) => stubs[id], querySelectorAll: () => [] },
    innerWidth: 1280,
  });
  vm.runInContext(declarations, context, { timeout: 2000 });
  const run = (code) => vm.runInContext(code, context, { timeout: 2000 });
  // Keep only real, already available setup and communication destinations.
  run(
    `tryInner=()=>'<span class="try-tag">'+L('Connect NyxBot','连接 NyxBot')+'</span><span class="try-txt">'+L('Sign in to connect your services and set up your assistant.','登录后连接服务，设置你的助手。')+'</span><span class="try-links"><a class="btn sm" href="#/signup">'+L('Get started','开始使用')+'</a></span>';`,
  );
  // chrome() only captures template strings into stubs; no prototype event handlers execute.
  run(
    `chrome=(${run("chrome.toString()").replace(/document.querySelectorAll\('\[data-lang\]'\)[\s\S]*$/, "}")}); chrome();`,
  );
  const entries = { nav: stubs.nav.innerHTML, footer: stubs.foot.innerHTML };
  for (const r of routes)
    entries[r] = run(`pages[${q(r === "" ? "home" : r)}]()`);
  for (const slug of run("SOL.map(s=>s.slug)"))
    entries["solutions/" + slug] = run(`pages.solution(${q(slug)})`);
  for (const slug of run("POSTS.map(s=>s.slug)"))
    entries["blog/" + slug] = run(`pages.post(${q(slug)})`);
  const integrations = run("INTEGRATIONS");
  if (locale === "en")
    fs.writeFileSync(
      path.join(outDir, "integrations.json"),
      JSON.stringify(
        integrations.map(([name, cat, available]) => ({
          name: name.toLowerCase(),
          cat: String(cat),
          available: String(available),
        })),
        null,
        2,
      ) + "\n",
    );
  const ucStart = source.indexOf("const __UC = "),
    ucEnd = source.indexOf("\nconst __esc", ucStart);
  const filmSource = vm.runInNewContext(source.slice(ucStart, ucEnd) + ";__UC");
  const filmScript = filmSource.slice(
    filmSource.lastIndexOf("<script>") + 8,
    filmSource.indexOf("const D = 6.6"),
  );
  const filmContext = {
    URLSearchParams,
    location: { search: "?lang=" + locale },
    window: {},
    document: { documentElement: {}, getElementById: () => null },
  };
  const filmData = vm.runInNewContext(filmScript + ";SCENES", filmContext);
  fs.writeFileSync(
    path.join(outDir, locale + "-film.json"),
    JSON.stringify(filmData, null, 2) + "\n",
  );
  const demos = run(
    `NB.map((n,i)=>({label:tx(n.chip),messages:n.convo.map(([w,t])=>({kind:w,text:tx(t)})),active:n.spec}))`,
  );
  const demoAgents = run(
    `NB_TEAM.map(si=>({name:tx(SOL[si].name),icon:SOL[si].icon}))`,
  );

  const banner = `// Generated from approved A, SHA-256 ${hash}.\n// Run: node scripts/website-a/generate.mjs <baseline.html>\n`;
  let output =
    banner +
    `import {lazy,useEffect} from 'react';\nimport {useWebsite} from '../website-context';\n`;
  fs.mkdirSync(path.join(outDir, locale), { recursive: true });
  const pageTitles = {};
  fs.writeFileSync(
    path.join(outDir, locale + "-demos.json"),
    JSON.stringify({ demos, agents: demoAgents }, null, 2) + "\n",
  );
  const exports = [];
  for (const [route, markup] of Object.entries(entries)) {
    const tree = html(markup);
    // Remove internal review links and prototype-only pending forms.
    if (route === "nav") {
      tree.querySelector(".links").insertAdjacentHTML(
        "beforeend",
        `<a class="nl m-only" href="#/login">${locale === "zh" ? "登录" : "Log in"}</a>`,
      );
    }
    tree.querySelectorAll(".ann").forEach((e) => e.remove());
    tree.querySelectorAll(".legal-links a").forEach((a, i) => {
      if (i === 2) a.remove();
      else a.setAttribute("href", ["/terms", "/privacy"][i]);
    });
    if (route === "agents") {
      const docs = [
        "/docs/ai/guides/mcp-proxy",
        "/docs/ai/guides/agent-isolation",
        "/docs/shared/concepts/credential-nodes",
      ];
      tree
        .querySelectorAll('.grid.g4 a[href="#/agents"]')
        .forEach((link, i) => {
          link.setAttribute("href", docs[i]);
          link.querySelector("p").textContent =
            locale === "zh" ? "阅读文档" : "Read the docs";
        });
    }
    if (route === "footer") {
      tree
        .querySelector(".fgrid > div:nth-child(4)")
        .insertAdjacentHTML(
          "beforeend",
          '<a href="/docs">Docs</a><a href="/docs/cli/getting-started/install">CLI</a>',
        );
    }
    const solGrid = tree.querySelector("#solGrid");
    if (solGrid) {
      solGrid.innerHTML = run("SOL.map(solCard).join('')");
      [...solGrid.children].forEach((el, i) => {
        const data = run(`SOL[${i}]`);
        el.dataset.category = data.cat;
        el.dataset.channels = data.ch.join(",");
      });
      solGrid.insertAdjacentHTML(
        "beforeend",
        run(
          `'<a class="card sol" href="#/nyxbot" style="border-style:dashed;align-items:center;justify-content:center;text-align:center"><div class="ic">'+ic('spark')+'</div><h3>'+L('Describe your own','描述你自己的需求')+'</h3><p>'+L('“When someone asks for a quote, collect their address and email it to me.”','“有人询价时，收集他们的地址并发邮件给我。”')+'</p><span class="btn sm" style="margin-top:6px">'+L('Ask NyxBot','交给 NyxBot')+'</span></a>'`,
        ),
      );
    }
    const blogGrid = tree.querySelector("#blogGrid");
    if (blogGrid) {
      blogGrid.innerHTML = run(
        `POSTS.filter(p=>!p.feat).map(postCard).join('')`,
      );
      [...blogGrid.children].forEach(
        (el, i) =>
          (el.dataset.category = run(`POSTS.filter(p=>!p.feat)[${i}].cat`)),
      );
    }
    const newsletter = tree.querySelector(".nl-form");
    if (newsletter) {
      newsletter.querySelector("input").setAttribute("disabled", "");
      const button = newsletter.querySelector("button");
      button.disabled = true;
      button.textContent =
        locale === "zh" ? "订阅暂未开放" : "Subscriptions coming soon";
    }
    if (route === "contact") {
      tree.querySelector("#cForm").remove();
      tree.querySelector("#cDone").remove();
      tree.querySelector(".bot").remove();
      const form = tree.querySelector(".contact-form");
      form.innerHTML = run(
        `'<div class="over">'+L('Contact','联系')+'</div><h2 class="h3">'+L('Talk to the team','与团队交流')+'</h2><p class="sub">'+L('Join our Discord for questions about bundles, self-hosting or your setup.','有关套餐、自托管或设置的问题，欢迎加入 Discord 与团队交流。')+'</p>'+discordBtn('btn p')`,
      );
    }
    tree.querySelectorAll("p").forEach((p) => {
      if (/\[Placeholder:|［占位：/.test(p.textContent))
        p.textContent =
          locale === "zh"
            ? "请联系团队确认当前的部署与认证信息。"
            : "Contact the team to confirm current hosting and certification details.";
    });
    tree.querySelectorAll(".callout").forEach((el) => {
      if (/prototype|原型/.test(el.textContent))
        el.textContent =
          locale === "zh"
            ? "示例场景，用于说明工作方式，并非真实客户案例。"
            : "Illustrative scenario showing how this works; not a customer case study.";
    });
    const component =
      route === "nav"
        ? "WebsiteHeader"
        : route === "footer"
          ? "WebsiteFooter"
          : (route || "home")
              .split(/[-/]/)
              .map((part) => part[0].toUpperCase() + part.slice(1))
              .join("") + "Page";
    const body =
      jsxChildren(tree) + (route === "" ? "<CommunityAccess />" : "");
    const interactions = [
      "AudienceSwitcher",
      "CodeExample",
      "DemoAgents",
      "DemoMessages",
      "DemoChips",
      "SolutionFilm",
      "CommunityAccess",
    ].filter((name) => body.includes("<" + name));
    const imports = interactions.length
      ? `import {${interactions.join(",")}} from '../../interactions';\n`
      : "";
    const pageSource =
      banner +
      imports +
      `import {useWebsite} from '../../website-context';\nexport default function ${component}(){const s=useWebsite(); void s; return <>${body}</>;}\n`;
    fs.writeFileSync(
      path.join(outDir, locale, component + ".tsx"),
      await format(pageSource, { parser: "typescript" }),
    );
    if (route === "nav" || route === "footer")
      output += `import ${component} from './${locale}/${component}';\n`;
    else
      output += `const ${component}=lazy(()=>import('./${locale}/${component}'));\n`;
    const heading = tree.querySelector("h1");
    if (heading) {
      heading
        .querySelectorAll("br")
        .forEach((br) => br.replaceWith(dom.document.createTextNode(" ")));
      pageTitles["/" + (route === "notfound" ? "404" : route)] =
        heading.textContent.replace(/\s+/g, " ").trim();
    }
    exports.push(
      `${q("/" + (route === "notfound" ? "404" : route))}:${component}`,
    );
  }
  output += `const pages={${exports.join(",")}};\nconst titles:Record<string,string>=${q(pageTitles)};\nexport default function WebsiteContent(){const s=useWebsite();const Page=(pages as Record<string,React.ElementType>)[s.route]??pages["/404"];useEffect(()=>{document.title=(titles[s.route]??titles['/404'])+' · NyxID';},[s.route]);return <><nav className={"nav "+(s.menuOpen?"open":"")} id="nav"><WebsiteHeader /></nav><main id="website-main" tabIndex={-1}><Page /></main><footer id="foot"><WebsiteFooter /></footer></>;}\n`;
  fs.writeFileSync(
    path.join(outDir, locale + ".tsx"),
    await format(output, { parser: "typescript" }),
  );
  if (locale === "en")
    fs.writeFileSync(
      path.join(outDir, "routes.json"),
      JSON.stringify(
        Object.keys(pageTitles).filter((route) => route !== "/404"),
        null,
        2,
      ) + "\n",
    );
}
// Scope every public rule; never leak body, button, input or theme styles into auth/app routes.
const css = source.match(/<style>([\s\S]*?)<\/style>/)[1];
const sheet = postcss.parse(css);
const keyframes = [];
sheet.walkAtRules(/keyframes$/, (rule) => {
  keyframes.push(rule.params);
  rule.params = "website-a-" + rule.params;
});
sheet.walkDecls(/^(animation|animation-name)$/, (decl) => {
  for (const name of keyframes)
    decl.value = decl.value.replace(
      new RegExp("\\b" + name + "\\b", "g"),
      "website-a-" + name,
    );
});
sheet.walkRules((rule) => {
  if (rule.parent.type === "atrule" && /keyframes/.test(rule.parent.name))
    return;
  rule.selectors = rule.selectors.map((sel) => {
    sel = sel
      .replace(/:root/g, ".website-a")
      .replace(/html\[lang="zh-CN"\]/g, '.website-a[lang="zh-CN"]')
      .replace(/\[data-theme="(dark|light)"\]/g, '.website-a[data-theme="$1"]')
      .replace(/(?<![.\w-])\b(?:html|body)\b/g, ".website-a");
    return sel.startsWith(".website-a") ? sel : ".website-a " + sel;
  });
});
fs.writeFileSync(
  path.join(root, "src/features/website-a/website-a.css"),
  '@import url("https://fonts.googleapis.com/css2?family=Mona+Sans:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&family=Noto+Sans+SC:wght@400;500;700&display=swap");\n' +
    sheet.toString() +
    `\n.website-a [hidden]{display:none!important}\n.website-a{min-height:100vh;font-size:15px;line-height:1.6;isolation:isolate}\n.website-a .ann{display:none}\n.website-a .icp-tabs button[aria-selected="true"]{background:var(--fg)!important;border-radius:999px}\n.website-a .icp-ind{display:none}\n.website-a .icp-progress.done span{transform:scaleX(1)}\n.website-a .nb-app{min-width:0}\n.website-a .demo-label{font-size:12px;color:var(--fg3);margin:8px 0}\n.website-a .nl-form :disabled{opacity:.65;cursor:not-allowed}\n.website-skip{position:absolute;left:16px;top:-100px;z-index:100}.website-skip:focus{top:12px;background:var(--card);padding:8px}\n.website-a .icp-panel.is-running .icp-progress span{animation:website-a-icpBar 9s linear forwards}.website-a .icp-panel.is-stopped .icp-progress span{transform:scaleX(1)}\n.website-a .audience-switcher:hover .icp-progress span{animation-play-state:paused}\n`,
);
console.log(
  "Generated native React A pages, scoped CSS and local brand assets.",
);
