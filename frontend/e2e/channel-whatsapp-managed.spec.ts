import { expect, test } from "@playwright/test";
import { managedBot, mockDashboard } from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  test(`managed Meta signup at ${String(viewport.width)}px`, async ({
    page,
  }) => {
    await page.setViewportSize(viewport);
    await mockDashboard(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.addInitScript(() => {
      window.FB = {
        init: (options) => {
          sessionStorage.setItem("meta-init", JSON.stringify(options));
        },
        login: (callback, options) => {
          sessionStorage.setItem("meta-options", JSON.stringify(options));
          const data = JSON.stringify({
            type: "WA_EMBEDDED_SIGNUP",
            event: "FINISH",
            data: { phone_number_id: "333", waba_id: "444" },
          });
          window.dispatchEvent(
            new MessageEvent("message", {
              origin: "https://www.facebook.com.attacker.test",
              data,
            }),
          );
          callback({ authResponse: { code: "one-use-code" } });
          window.setTimeout(
            () =>
              window.dispatchEvent(
                new MessageEvent("message", {
                  origin: "https://www.facebook.com",
                  data,
                }),
              ),
            250,
          );
        },
      };
    });
    await page.route("**/channel-bots/managed-onboarding/whatsapp", (route) =>
      route.fulfill({
        json: {
          available: true,
          app_id: "111",
          embedded_signup_config_id: "222",
          graph_version: "v25.0",
          feature_types: ["", "whatsapp_business_app_onboarding"],
        },
      }),
    );
    let completion: unknown;
    await page.route(
      "**/channel-bots/managed-onboarding/whatsapp/complete",
      async (route) => {
        completion = route.request().postDataJSON();
        await route.fulfill({
          contentType: "text/event-stream",
          body: [
            { stage: "exchanging" },
            { stage: "subscribing" },
            { stage: "registering" },
            { result: managedBot },
          ]
            .map((value) => `data: ${JSON.stringify(value)}\n\n`)
            .join(""),
        });
      },
    );
    await page.goto("/channel-bots?connect=whatsapp");
    const dialog = page.getByRole("dialog");
    await expect(
      dialog.getByRole("button", { name: "Connect with Meta" }),
    ).toBeVisible();
    await expect(
      dialog.getByLabel("Access Token", { exact: true }),
    ).toHaveCount(0);
    await dialog.getByLabel("Label", { exact: true }).fill(managedBot.label);
    await dialog
      .getByLabel("I already use the WhatsApp Business app on this number")
      .check();
    await dialog.screenshot({
      path: `/tmp/nyx-managed-whatsapp-${String(viewport.width)}.png`,
    });
    expect(
      await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
    ).toBe(true);
    await dialog.getByRole("button", { name: "Connect with Meta" }).click();
    await expect(page).toHaveURL(/channel-bots\/managed-whatsapp$/);
    expect(completion).toEqual({
      code: "one-use-code",
      phone_number_id: "333",
      waba_id: "444",
      label: managedBot.label,
    });
    const options = await page.evaluate(() =>
      JSON.parse(sessionStorage.getItem("meta-options") ?? "{}"),
    );
    expect(options).toEqual({
      config_id: "222",
      response_type: "code",
      override_default_response_type: true,
      extras: {
        setup: {},
        featureType: "whatsapp_business_app_onboarding",
        sessionInfoVersion: "3",
      },
    });
    await expect(
      page.getByText("Platform-managed", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Finish webhook setup", { exact: true })).toHaveCount(0);
    await expect(page.getByText("Waiting for the first verified inbound message.", { exact: true })).toBeVisible();
    await expect(
      page.getByLabel("Meta App Secret", { exact: true }),
    ).toHaveCount(0);
    await expect(
      page.getByRole("button", { name: "Re-register number" }),
    ).toBeVisible();
    await page.screenshot({
      path: `/tmp/nyx-managed-detail-${String(viewport.width)}.png`,
      fullPage: true,
    });
    expect(errors).toEqual([]);
  });
}

test("managed cancellation and Advanced preserve the BYO form", async ({
  page,
}) => {
  await mockDashboard(page);
  await page.addInitScript(() => {
    window.FB = {
      init: () => {},
      login: () =>
        window.dispatchEvent(
          new MessageEvent("message", {
            origin: "https://www.facebook.com",
            data: {
              type: "WA_EMBEDDED_SIGNUP",
              event: "CANCEL",
              data: { current_step: "business_selection" },
            },
          }),
        ),
    };
  });
  await page.route("**/channel-bots/managed-onboarding/whatsapp", (route) =>
    route.fulfill({
      json: {
        available: true,
        app_id: "111",
        embedded_signup_config_id: "222",
        graph_version: "v25.0",
        feature_types: [""],
      },
    }),
  );
  await page.goto("/channel-bots?connect=whatsapp&label=Support");
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Connect with Meta" }).click();
  await expect(
    dialog.getByText("Meta signup cancelled at business_selection."),
  ).toBeVisible();
  await dialog
    .getByText("Advanced: use your own Meta app", { exact: true })
    .click();
  await expect(
    dialog.getByLabel("Access Token", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByLabel("Meta App Secret", { exact: true }),
  ).toBeVisible();
});
