import { expect, test } from "@playwright/test";
import { mockDashboard } from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`admin platform credentials at ${String(viewport.width)}px`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await mockDashboard(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const provider = {
      provider: "meta",
      label: "Meta",
      platform: "whatsapp",
      available: false,
      updated_at: null as string | null,
      callback_url:
        "https://nyxid.example/api/v1/webhooks/channel/whatsapp/platform",
      webhook_verify_token: null as string | null,
      setup_checklist: [
        "Add the WhatsApp product to your Meta app.",
        "Request advanced access for whatsapp_business_management and whatsapp_business_messaging.",
      ],
      fields: [
        {
          name: "app_id",
          label: "App ID",
          secret: false,
          help: "Meta App Dashboard",
          required: true,
          numeric: true,
          configured: false,
          value: "",
        },
        {
          name: "app_secret",
          label: "App Secret",
          secret: true,
          help: "Meta App Dashboard > Basic",
          required: true,
          numeric: false,
          configured: false,
        },
        {
          name: "embedded_signup_config_id",
          label: "Embedded Signup Configuration ID",
          secret: false,
          help: "Facebook Login for Business",
          required: true,
          numeric: true,
          configured: false,
          value: "",
        },
      ],
    };
    const updates: unknown[] = [];
    await page.route("**/api/v1/admin/platform-credentials", (route) =>
      route.fulfill({ json: [provider] }),
    );
    await page.route(
      "**/api/v1/admin/platform-credentials/meta",
      async (route) => {
        const body = route.request().postDataJSON() as {
          fields?: Record<string, string | null>;
          regenerate_verify_token?: boolean;
        };
        updates.push(body);
        for (const field of provider.fields) {
          if (Object.hasOwn(body.fields ?? {}, field.name)) {
            field.configured = body.fields?.[field.name] !== null;
            if (!field.secret) field.value = body.fields?.[field.name] ?? "";
          }
        }
        provider.available = provider.fields.every((field) => field.configured);
        provider.updated_at = new Date().toISOString();
        provider.webhook_verify_token = body.regenerate_verify_token
          ? "replacement-verify-token"
          : "generated-verify-token";
        await route.fulfill({ json: provider });
      },
    );
    await page.goto("/admin/platform-credentials");
    await page.getByLabel("App ID", { exact: true }).fill("111");
    await page
      .getByLabel("App Secret", { exact: true })
      .fill("private-secret-never-returned");
    await page
      .getByLabel("Embedded Signup Configuration ID", { exact: true })
      .fill("222");
    await page.getByRole("button", { name: "Save credentials" }).click();
    await expect(page.getByLabel("App Secret", { exact: true })).toHaveValue(
      "",
    );
    await expect(
      page.getByRole("button", { name: "Copy Platform Verify Token" }),
    ).toBeVisible();
    expect(updates[0]).toEqual({
      fields: {
        app_id: "111",
        app_secret: "private-secret-never-returned",
        embedded_signup_config_id: "222",
      },
    });
    await page.getByRole("button", { name: "Regenerate verify token" }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Confirm", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toBeHidden();
    await expect(
      page.getByText("replacement-verify-token", { exact: true }),
    ).toBeVisible();
    await page.screenshot({
      path: `/tmp/nyx-admin-credentials-${String(viewport.width)}.png`,
      fullPage: true,
    });
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    expect(errors).toEqual([]);
  });
}
