import fs from "node:fs/promises";
import ts from "typescript";

// Read route declarations as data; never execute the private application in the renderer.
export function applicationPaths(source) {
  const ast = ts.createSourceFile(
      "router.tsx",
      source,
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    ),
    routes = new Map();
  function visit(node) {
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name)) {
      let call = node.initializer;
      if (call && ts.isConditionalExpression(call)) call = call.whenTrue;
      if (
        call &&
        ts.isCallExpression(call) &&
        call.expression.getText(ast) === "createRoute"
      ) {
        const obj = call.arguments[0];
        if (!obj || !ts.isObjectLiteralExpression(obj))
          throw new Error("Unsupported route declaration");
        const props = new Map(
          obj.properties
            .filter(ts.isPropertyAssignment)
            .map((p) => [p.name.getText(ast), p.initializer]),
        );
        const path = props.get("path"),
          parent = props.get("getParentRoute");
        if (path && !ts.isStringLiteral(path))
          throw new Error("Nonliteral application route path");
        const parentName =
          parent && ts.isArrowFunction(parent) && ts.isIdentifier(parent.body)
            ? parent.body.text
            : "rootRoute";
        routes.set(node.name.text, {
          path: path?.text ?? "",
          parent: parentName,
        });
      }
    }
    ts.forEachChild(node, visit);
  }
  visit(ast);
  function full(name, depth = 0) {
    if (name === "rootRoute") return "";
    if (depth > 8 || !routes.has(name))
      throw new Error(`Unknown/cyclic parent: ${name}`);
    const route = routes.get(name);
    return `${full(route.parent, depth + 1)}/${route.path.replace(/^\//, "")}`
      .replace(/\/+/g, "/")
      .replace(/\/$/, "");
  }
  return [
    ...new Set(
      [...routes.keys()]
        .filter((name) => name !== "oauthConsentPreviewRoute")
        .map((name) => full(name) || "/"),
    ),
  ];
}
const headers = `include @@STATIC_HEADERS@@;`;
export async function writeRouting(root, dist, pages) {
  const app = applicationPaths(
    await fs.readFile(`${root}/src/router.tsx`, "utf8"),
  ).filter(
    (p) =>
      p !== "/" &&
      !/^\/(?:zh|nyxbot$|solutions|how-it-works|security|agents|integrations|pricing|contact|blog|docs|preview|privacy|terms)(?:\/|$)/.test(
        p,
      ),
  );
  // In particular, no unknown /admin/* or /settings/* SPA catch-all.
  const patterns = app.map((p) =>
    p
      .split("/")
      .map((s) =>
        s.startsWith("$")
          ? s === "$"
            ? ".+"
            : "[^/]+"
          : s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
      )
      .join("/"),
  );
  const maps = `map $host $nyxid_preview_robots { nyx.chrono-ai.fun ""; default "noindex, nofollow"; }
map $args $nyxid_repeated_lang { default 0; ~*(^|&)lang=[^&]*(&[^&]*)*&lang= 1; }
map $args $nyxid_clean_args {
 default $args;
 ~*^lang=[^&]*&?(?<nyxid_lang_tail>.*)$ $nyxid_lang_tail;
 ~*^(?<nyxid_lang_lead>.+)&lang=[^&]*&(?<nyxid_lang_rest>.+)$ "$nyxid_lang_lead&$nyxid_lang_rest";
 ~*^(?<nyxid_lang_prefix>.+)&lang=[^&]*$ $nyxid_lang_prefix;
}
map $nyxid_clean_args $nyxid_locale_query { "" ""; default "?$nyxid_clean_args"; }
map "$uri:$arg_lang" $nyxid_public_redirect {
 default "";
${pages
  .filter((p) => p.translated)
  .flatMap((p) => [p.path, ...(p.path === "/" ? [] : [p.path + "/"])])
  .flatMap((uri) => {
    const route = uri.replace(/^\/zh(?=\/|$)/, "").replace(/\/$/, "") || "/";
    return [
      ` "${uri}:en" "${route}";`,
      ` "${uri}:zh" "/zh${route === "/" ? "" : route}";`,
    ];
  })
  .join("\n")}
}
`;
  await fs.writeFile(`${dist}/public-routing-maps.conf`, maps);
  const locations = pages
    .map(
      (p) => `location = ${p.path} {
 ${p.translated ? "if ($nyxid_repeated_lang) { return 400; }\n if ($nyxid_public_redirect) { return 301 $nyxid_public_redirect$nyxid_locale_query; }" : ""}
 if ($request_method !~ ^(GET|HEAD)$) { return 405; }
 default_type text/html;
 ${["/privacy", "/terms"].includes(p.path) ? 'etag off;\n if_modified_since off;\n if ($http_if_none_match = $nyxid_legal_etag) { return 304; }\n add_header ETag $nyxid_legal_etag;' : ''}
 add_header X-Robots-Tag $nyxid_preview_robots always;
 ${headers}
 add_header Cache-Control "${["/privacy", "/terms"].includes(p.path) ? "public, max-age=86400, must-revalidate" : "no-cache"}" always;
 try_files /${p.file} =404;
}
${
  p.path === "/"
    ? ""
    : `location = ${p.path}/ {
 ${headers}
 ${p.translated ? "if ($nyxid_repeated_lang) { return 400; }\n if ($nyxid_public_redirect) { return 301 $nyxid_public_redirect$nyxid_locale_query; }" : ""}
 return 301 ${p.path}$is_args$args;
}`
}`,
    )
    .join("\n");
  const mirrors = [
    ...pages.map((p) => p.markdown),
    ...pages
      .filter((p) => p.path.startsWith("/docs/"))
      .map((p) => `${p.path}.md`),
    "/legal/privacy.md",
    "/legal/terms.md",
    "/read/en/index.md",
    "/read/zh/index.md",
  ];
  await fs.writeFile(
    `${dist}/public-routing.conf`,
    `${locations}
location = /privacy.html { return 301 /privacy$is_args$args; }
location = /terms.html { return 301 /terms$is_args$args; }
location ^~ /__public/ { internal; }
location = /public-routing.conf { return 404; }
location = /public-routing-maps.conf { return 404; }
location = /legal-cache.conf { return 404; }
location = /app.html { internal; }
location = /index.html { return 404; }
${mirrors.map((p) => `location = ${p} { default_type "text/markdown; charset=utf-8"; ${headers} add_header X-Robots-Tag "noindex" always; add_header Cache-Control "no-cache" always; try_files $uri =404; }`).join("\n")}
location = /llms.txt { default_type "text/plain; charset=utf-8"; ${headers} add_header Cache-Control "no-cache" always; try_files $uri =404; }
location ~ ^(?:${patterns.join("|")})/?$ {
 ${headers}
 add_header X-Robots-Tag "noindex, nofollow" always;
 add_header Cache-Control "no-store" always;
 try_files /app.html =404;
}
`,
  );
  await fs.writeFile(
    `${dist}/application-routes.json`,
    JSON.stringify(app, null, 2),
  );
}
