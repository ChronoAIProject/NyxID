import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";

test("standalone built wizard loads connection and platform scope through the CLI shim without dashboard auth", async ({
  page,
}) => {
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
  expect(requests).toContain("/api/proxy/api/v1/keys");
  expect(escaped).toEqual([]);
  expect(requests.some((path) => path.endsWith("/users/me"))).toBe(false);
});
