import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

for (const width of [1280, 390]) {
  test(`standalone translated wizard preserves scope and stays within the CLI shim at ${width}px`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const bootstrap = {
      flow: "api-key-create",
      context: "local",
      csrf: "isolated-wizard-csrf",
      baseUrl: "https://backend.example",
      prefill: { name: "Scope regression", scopes: "proxy" },
    };
    const bundle = await readFile(
      new URL("../../cli/src/wizard/assets/index.html", import.meta.url),
      "utf8",
    );
    const html = bundle.replace(
      "<head>",
      `<head><script>window.__WIZARD_BOOTSTRAP__ = ${JSON.stringify(bootstrap)};</script>`,
    );
    const requests: string[] = [];
    const escaped: string[] = [];
    await page.route("**/api/v1/**", async (route) => {
      const request = route.request();
      const path = new URL(request.url()).pathname;
      requests.push(path);
      if (!path.startsWith("/api/proxy/")) escaped.push(path);
      expect(request.headers()["x-wizard-csrf"]).toBe(bootstrap.csrf);
      const data = path.endsWith("/keys")
        ? {
            keys: [
              {
                id: "connection",
                label: "Wizard Slack",
                slug: "api-slack",
                is_active: true,
                auto_connected: false,
                credential_source: { type: "personal" },
              },
              {
                id: "platform",
                label: "Wizard platform",
                slug: "llm-openai",
                is_active: true,
                auto_connected: true,
                credential_source: { type: "personal" },
              },
            ],
          }
        : path.endsWith("/nodes")
          ? { nodes: [] }
          : { orgs: [] };
      await route.fulfill({ json: data });
    });
    await page.route("**/api/proxy/heartbeat", (route) =>
      route.fulfill({ json: { ok: true } }),
    );
    await page.route("**/isolated-wizard", (route) =>
      route.fulfill({ contentType: "text/html", body: html }),
    );
    await page.goto("/isolated-wizard");
    await expect(
      page.getByRole("heading", { name: "Create an API key" }),
    ).toBeVisible();
    await page
      .getByRole("checkbox", { name: "Allow all services", exact: true })
      .click();
    const connection = page.getByRole("checkbox", { name: /Wizard Slack/ });
    await expect(connection).toBeVisible();
    await connection.click();
    await expect(connection).toBeChecked();
    const platform = page.getByRole("checkbox", {
      name: "Wizard platform",
      exact: true,
    });
    await expect(platform).toBeVisible();
    await platform.click();
    await expect(platform).toBeChecked();
    await page.getByLabel("Name", { exact: true }).fill("Keep my draft 保留");
    await page
      .getByRole("combobox", { name: "Language", exact: true })
      .click();
    await page.getByRole("option", { name: "简体中文", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "创建 API 密钥", exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("名称", { exact: true })).toHaveValue(
      "Keep my draft 保留",
    );
    await expect(connection).toBeChecked();
    await expect(platform).toBeChecked();
    await page
      .getByRole("combobox", { name: "语言", exact: true })
      .click();
    await page.getByRole("option", { name: "繁體中文", exact: true }).click();
    await expect(
      page.getByRole("heading", { name: "建立 API 金鑰", exact: true }),
    ).toBeVisible();
    await expect(page.getByLabel("名稱", { exact: true })).toHaveValue(
      "Keep my draft 保留",
    );
    await expect(connection).toBeChecked();
    await expect(platform).toBeChecked();
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: testInfo.outputPath(`wizard-zh-TW-${width}.png`),
    });
    await page.reload();
    await expect(
      page.getByRole("combobox", { name: "語言", exact: true }),
    ).toHaveText("ZH繁體中文");
    await expect(
      page.getByRole("heading", { name: "建立 API 金鑰", exact: true }),
    ).toBeVisible();
    expect(requests).toContain("/api/proxy/api/v1/keys");
    expect(escaped).toEqual([]);
    expect(requests.some((path) => path.endsWith("/users/me"))).toBe(false);
  });
}
