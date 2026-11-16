import { expect, type Locator, type Page } from "@playwright/test";

export async function disclosureIcons(container: Locator, root: boolean) {
  const rows = container.locator('button[data-drawer-page-trigger]:visible:not([inert] *, [aria-hidden="true"] *)');
  const icons: string[] = [];
  for (const row of await rows.all()) {
    const leading = row.locator(":scope > span:first-child > svg");
    await expect(leading).toHaveCount(root ? 1 : 0);
    if (root) icons.push((await leading.getAttribute("class"))!);
  }
  if (root) expect(new Set(icons).size).toBe(icons.length);
}

export async function settingsApi(page: Page) {
  const writes: { method: string; path: string; body: unknown }[] = [];
  let user = {
    id: "d4f5a6b7-c8d9-4e0f-a1b2-c3d4e5f60718",
    display_name: "Reader",
    email:
      "reader.account-settings-review+long-address-for-verification@international-research-department.example.com",
    email_verified: true,
    has_password: true,
    mfa_enabled: false,
    is_admin: false,
    is_active: true,
    created_at: "2026-01-01T00:00:00Z",
    capabilities: { enabled_features: ["assistant:nyxagent-engine"] },
    profile_config: {
      security: { skip_mfa_for_approved_sign_ins: true },
      onboarding: { ai_services_completed_at: "2026-01-01T00:00:00Z" },
    },
  };
  await page.route("**/src/lib/mock-data.ts*", async (route) => {
    const response = await route.fetch();
    const source = await response.text();
    const signature = 'export function getMockResponse(endpoint, method = "GET", body) {';
    expect(source).toContain(signature);
    await route.fulfill({
      response,
      body: source.replace(
        signature,
        `${signature}
      if (["/users/me", "/auth/password/change", "/sessions"].includes(endpoint.split("?")[0]) || endpoint.startsWith("/auth/mfa/")) return undefined;`,
      ),
    });
  });
  await page.route("**/api/v1/users/me", async (route) => {
    if (route.request().method() === "PUT") {
      const body = route.request().postDataJSON();
      writes.push({ method: "PUT", path: "/users/me", body });
      user = { ...user, ...body };
    }
    await route.fulfill({ json: user });
  });
  await page.route("**/api/v1/auth/password/change", async (route) => {
    writes.push({
      method: route.request().method(),
      path: "/auth/password/change",
      body: route.request().postDataJSON(),
    });
    await route.fulfill({ json: {} });
  });
  await page.route("**/api/v1/sessions", (route) =>
    route.fulfill({
      json: [
        {
          id: "current",
          is_current: true,
          user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) Chrome/128",
          ip_address: "203.0.113.1",
          created_at: "2026-11-01T00:00:00Z",
          expires_at: "2026-12-01T00:00:00Z",
        },
        {
          id: "phone",
          is_current: false,
          user_agent:
            "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) Version/18.0 Mobile Safari/604.1",
          ip_address: "203.0.113.2",
          created_at: "2026-11-02T00:00:00Z",
          expires_at: "2026-12-02T00:00:00Z",
        },
      ],
    }),
  );
  await page.route("**/api/v1/auth/mfa/setup", (route) =>
    route.fulfill({
      json: {
        secret: "JBSWY3DPEHPK3PXP",
        qr_code_url: "otpauth://totp/NyxID?secret=JBSWY3DPEHPK3PXP",
      },
    }),
  );
  await page.route("**/api/v1/auth/mfa/confirm", (route) => {
    writes.push({
      method: route.request().method(),
      path: "/auth/mfa/confirm",
      body: route.request().postDataJSON(),
    });
    user = { ...user, mfa_enabled: true };
    return route.fulfill({
      json: { recovery_codes: ["recovery-one", "recovery-two"] },
    });
  });
  await page.route("**/api/v1/auth/mfa/disable", (route) => {
    writes.push({
      method: route.request().method(),
      path: "/auth/mfa/disable",
      body: route.request().postDataJSON(),
    });
    user = { ...user, mfa_enabled: false };
    return route.fulfill({ json: {} });
  });
  return writes;
}

export async function sectionAddress(
  page: Page,
  pathname: string,
  search: Record<string, unknown>,
  replace = true,
) {
  await page.evaluate(
    async ({ pathname, search, replace }) => {
      const { router } = (
        window as unknown as {
          __nyxApp: {
            router: {
              state: { location: { search: { mock?: number } } };
              navigate: (options: unknown) => Promise<void>;
            };
          };
        }
      ).__nyxApp;
      await router.navigate({
        to: pathname,
        search: { ...(router.state.location.search.mock ? { mock: 1 } : {}), ...search },
        replace,
      });
    },
    { pathname, search, replace },
  );
}
export async function clarityTheme(page: Page, width: number, mode: "light" | "dark") {
  await page.setViewportSize({ width, height: 900 });
  await page.addInitScript(
    (mode) =>
      localStorage.setItem(
        "nyxid.theme",
        JSON.stringify({ version: 2, state: { mode, textScale: 1, density: 1 } }),
      ),
    mode,
  );
  await page.route("**/api/v1/assistant/nyxagent/cma-sessions?**", (route) =>
    route.fulfill({ json: { sessions: [] } }),
  );
  await page.route("**/api/v1/assistant/nyxagent/machines?**", (route) =>
    route.fulfill({ json: [] }),
  );
  await page.route("**/api/v1/saved-logins?**", (route) => route.fulfill({ json: [] }));
  await page.route("**/api/v1/service-preferences", (route) =>
    route.fulfill({ json: { groups: [], version: 0, updated_at: null } }),
  );
  await page.route("**/api/v1/triggers", (route) => route.fulfill({ json: { triggers: [] } }));
  await page.route("**/api/v1/onboarding/bindings", (route) =>
    route.fulfill({ json: { bindings: [] } }),
  );
}

export async function accountSection(page: Page, section: string) {
  await page.evaluate(async (section) => {
    const { router } = (
      window as unknown as {
        __nyxApp: {
          router: {
            state: { location: { pathname: string; search: Record<string, unknown> } };
            navigate: (options: unknown) => Promise<void>;
          };
        };
      }
    ).__nyxApp;
    const { pathname, search } = router.state.location;
    await router.navigate({
      to: pathname,
      search: { ...search, [pathname === "/settings" ? "tab" : "panelTab"]: section },
      replace: true,
    });
  }, section);
}

export async function billingSection(page: Page, section: string) {
  await page.evaluate(async (section) => {
    const { router } = (
      window as unknown as {
        __nyxApp: {
          router: {
            state: { location: { pathname: string; search: Record<string, unknown> } };
            navigate: (options: unknown) => Promise<void>;
          };
        };
      }
    ).__nyxApp;
    const { pathname, search } = router.state.location;
    await router.navigate({
      to: pathname,
      search: { ...search, tab: section.split("/")[0], section },
      replace: true,
    });
  }, section);
}
