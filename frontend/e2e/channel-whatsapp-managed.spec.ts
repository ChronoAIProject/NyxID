import { expect, test } from "@playwright/test";
import {
  managedBot,
  managedBootstrap,
  mockDashboard,
} from "./managed-onboarding-fixtures";

for (const viewport of [
  { width: 1440, height: 1000 },
  { width: 390, height: 844 },
]) {
  for (const coexistence of [false, true]) {
    test(`managed Meta ${coexistence ? "signup" : "Cloud API signup"} at ${String(viewport.width)}px`, async ({
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
              event: options.extras.featureType
                ? "FINISH_WHATSAPP_BUSINESS_APP_ONBOARDING"
                : "FINISH",
              data: options.extras.featureType
                ? { waba_id: "444" }
                : {
                    phone_number_id: "333",
                    waba_id: "444",
                    business_id: "555",
                    page_ids: ["777"],
                  },
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
          json: managedBootstrap,
        }),
      );
      let completion: unknown;
      await page.route(
        "**/channel-bots/managed-onboarding/whatsapp/complete",
        async (route) => {
          completion = route.request().postDataJSON();
          expect(route.request().url()).toBe(
            "https://api.nyxid.example/api/v1/channel-bots/managed-onboarding/whatsapp/complete",
          );
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
      if (coexistence)
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
        ...(coexistence ? {} : { phone_number_id: "333", business_id: "555" }),
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
        extras: coexistence
          ? { featureType: "whatsapp_business_app_onboarding" }
          : {},
      });
      await expect(
        page.getByText("Platform-managed", { exact: true }),
      ).toBeVisible();
      await expect(
        page.getByText("Finish webhook setup", { exact: true }),
      ).toHaveCount(0);
      await expect(
        page.getByText("Waiting for the first verified inbound message.", {
          exact: true,
        }),
      ).toBeVisible();
      await expect(
        page.getByLabel("Meta App Secret", { exact: true }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Re-register number" }),
      ).toBeVisible();
      let repaired = false;
      await page.route(
        "**/channel-bots/managed-whatsapp/managed-setup/repair",
        async (route) => {
          repaired = true;
          await route.fulfill({
            json: {
              ...managedBot.managed_setup,
              webhook_override: "configured",
            },
          });
        },
      );
      await page.getByRole("button", { name: "Repair setup" }).click();
      await expect.poll(() => repaired).toBe(true);
      await page.getByRole("button", { name: "Delete", exact: true }).click();
      await expect(
        page
          .getByRole("dialog")
          .getByText(/number stays subscribed to the app in Meta/),
      ).toBeVisible();
      await page
        .getByRole("dialog")
        .getByRole("button", { name: "Cancel" })
        .click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      await page.screenshot({
        path: `/tmp/nyx-managed-detail-${String(viewport.width)}.png`,
        fullPage: true,
      });
      expect(errors).toEqual([]);
    });
  }
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
      json: { ...managedBootstrap, feature_types: [""] },
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
