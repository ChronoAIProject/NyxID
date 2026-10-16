// Reuse the existing public hero scene; deployment overrides are build-time configuration.
export const visualConfig = {
  enabled: import.meta.env.VITE_WEBSITE_VISUALS_ENABLED !== "false",
  backgroundUrl:
    import.meta.env.VITE_WEBSITE_BACKGROUND_RUNTIME_URL ||
    "https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/unicornStudio.umd.js",
  projectId:
    import.meta.env.VITE_WEBSITE_BACKGROUND_PROJECT_ID ||
    "sqyz3r4WjVjdLal6IHvt",
};
