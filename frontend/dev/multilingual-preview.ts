import type { Plugin } from "vite";

/** Explicitly enabled local previews; these routes are absent from production. */
export function multilingualPreview(): Plugin {
  return {
    name: "multilingual-preview",
    apply: "serve",
    transformIndexHtml(_html, context) {
      const url = new URL(
        context.originalUrl ?? context.path,
        "http://localhost",
      );
      return [
        {
          tag: "script",
          attrs: { type: "module" },
          children:
            'import { useThemeStore } from "/src/stores/theme-store.ts"; useThemeStore.getState().setTextScale(1);',
          injectTo: "head-prepend",
        },
        ...(url.searchParams.get("mock") === "1"
          ? [
              {
                tag: "script",
                children:
                  "globalThis.__nyxidAssistantHttpFaults = { nyxagentEnabled: true };",
                injectTo: "head-prepend" as const,
              },
            ]
          : []),
      ];
    },
    configureServer(server) {
      server.middlewares.use(async (req, res, next) => {
        const url = new URL(req.url ?? "/", "http://localhost");
        if (url.pathname !== "/__preview/wizard") return next();
        const html = await server.transformIndexHtml(
          req.url ?? "/",
          `<!doctype html>
<html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>NyxID wizard preview · sample data</title></head>
<body><div id="wizard-root"></div><script type="module" src="/src/dev/wizard-preview.ts"></script></body></html>`,
        );
        res.statusCode = 200;
        res.setHeader("Content-Type", "text/html; charset=utf-8");
        res.end(html);
      });
    },
  };
}
