import { expect, test } from "@playwright/test";
import type { ChannelMessageItem } from "../src/types/channels";
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
        dialog.getByLabel("Access token", { exact: true }),
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
      const deleteRequests: string[] = [];
      page.on("request", (request) => {
        if (request.method() === "DELETE") deleteRequests.push(request.url());
      });
      await page.getByRole("button", { name: "Delete", exact: true }).click();
      await expect(
        page
          .getByRole("dialog")
          .getByText(
            /This deletes the NyxID connection and its conversation routes/,
          ),
      ).toBeVisible();
      await expect(page.getByRole("dialog")).toContainText(
        "The bot remains on the messaging platform. Reconnecting requires assigning its agents again.",
      );
      await expect(page.getByRole("dialog")).toContainText(
        "The platform account stays connected. Manage it separately in your account's connections.",
      );
      await page
        .getByRole("dialog")
        .getByRole("button", { name: "Cancel" })
        .click();
      await expect(page.getByRole("dialog")).toHaveCount(0);
      expect(deleteRequests).toEqual([]);
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
  const completionRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/managed-onboarding/whatsapp/complete")) {
      completionRequests.push(request.url());
    }
  });
  await page.goto("/channel-bots?connect=whatsapp&label=Support");
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Connect with Meta" }).click();
  await expect(
    dialog.getByText("Meta signup cancelled at business_selection."),
  ).toBeVisible();
  await dialog
    .getByText("Advanced: use your own credentials", { exact: true })
    .click();
  await expect(
    dialog.getByLabel("Access token", { exact: true }),
  ).toBeVisible();
  await expect(
    dialog.getByLabel("Meta App Secret", { exact: true }),
  ).toBeVisible();
  await expect(dialog.getByLabel("Label", { exact: true })).toHaveValue(
    "Support",
  );
  await expect(dialog.getByLabel("Access token", { exact: true })).toHaveValue(
    "",
  );
  await expect(
    dialog.getByLabel("Meta App Secret", { exact: true }),
  ).toHaveValue("");
  await expect(
    dialog.getByRole("button", { name: "Add Bot", exact: true }),
  ).toBeDisabled();
  expect(completionRequests).toEqual([]);
});

for (const width of [1440, 390]) {
  test(`WhatsApp callback and delivery receipts wrap at ${String(width)}px`, async ({
    page,
  }) => {
    await page.setViewportSize({ width, height: 844 });
    await mockDashboard(page);
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const base: ChannelMessageItem = {
      id: "inbound",
      channel_bot_id: managedBot.id,
      conversation_id: "conversation",
      direction: "inbound",
      platform: "whatsapp",
      platform_message_id: "wamid.inbound",
      sender_platform_id: "15551234567",
      sender_display_name: null,
      content_type: "text",
      agent_api_key_id: "agent",
      callback_status: "delivered",
      reply_to_message_id: null,
      created_at: managedBot.created_at,
    };
    const messages: ChannelMessageItem[] = [
      ...(["delivered", "pending", "failed", "timeout"] as const).map(
        (status) => ({
          ...base,
          id: status,
          callback_status: status,
        }),
      ),
      {
        ...base,
        id: "outbound",
        direction: "outbound",
        callback_status: null,
        platform_message_id: `wamid.${"x".repeat(240)}`,
      },
    ];
    for (const status of [
      "delivered",
      "read",
      "failed",
      "partial",
      "unknown",
      "legacy_final_only",
    ] as const) {
      messages.push({
        ...base,
        id: `receipt-${status}`,
        direction: "outbound",
        callback_status: null,
        platform_message_id: status === "unknown" ? null : `wamid.${status}`,
        delivery: {
          status,
          complete: ["delivered", "read", "failed"].includes(status),
          expected_components:
            status === "legacy_final_only"
              ? null
              : status === "partial"
                ? 2
                : 1,
          recipient_only: false,
          failure_code:
            status === "partial" || status === "unknown" ? 10005 : null,
          components:
            status === "unknown" || status === "legacy_final_only"
              ? []
              : [
                  {
                    platform_message_id: `wamid.${status}.${"x".repeat(480)}`,
                    status: status === "partial" ? "read" : status,
                    sent_at: base.created_at,
                    delivered_at: status === "failed" ? null : base.created_at,
                    read_at:
                      status === "read" || status === "partial"
                        ? base.created_at
                        : null,
                    played_at: null,
                    failed_at: status === "failed" ? base.created_at : null,
                    error_codes: status === "failed" ? [131026] : [],
                  },
                ],
        },
      });
    }
    await page.route("**/channel-conversations/conversation", (route) =>
      route.fulfill({
        json: {
          id: "conversation",
          channel_bot_id: managedBot.id,
          platform: "whatsapp",
          platform_conversation_id: "15551234567",
          platform_conversation_type: "private",
          platform_sender_id: null,
          agent_api_key_id: "agent",
          default_agent: false,
          is_active: true,
          allow_agent_initiated: false,
          last_message_at: null,
          created_at: managedBot.created_at,
          updated_at: managedBot.updated_at,
        },
      }),
    );
    await page.route(
      "**/channel-conversations/conversation/messages?*",
      (route) =>
        route.fulfill({
          json: { messages, total: messages.length, page: 1, per_page: 50 },
        }),
    );
    await page.goto(
      `/channel-bots/${managedBot.id}/conversations/conversation`,
    );
    for (const label of [
      "Callback accepted",
      "Callback pending",
      "Callback failed",
      "Callback timed out",
      "Platform accepted",
      "Delivered",
      "Read",
      "Delivery failed",
      "Partially accepted",
      "Acceptance unknown",
      "Final part accepted",
    ]) {
      const badge = page.getByText(label, { exact: true });
      await expect(badge).toBeVisible();
      expect(
        await badge
          .locator("..")
          .evaluate(
            (footer) =>
              footer.scrollWidth <= footer.clientWidth &&
              footer.parentElement!.scrollWidth <=
                footer.parentElement!.clientWidth,
          ),
      ).toBe(true);
    }
    await expect(
      page.getByText(/Acceptance does not confirm recipient delivery/),
    ).toBeVisible();
    await expect(
      page.getByText("1 of 2 component IDs recorded."),
    ).toBeVisible();
    await expect(
      page.getByText(/Delivery of the whole reply cannot be confirmed/),
    ).toBeVisible();
    for (const summary of await page
      .locator("summary")
      .filter({ hasText: "Component receipts" })
      .all()) {
      await summary.click();
    }
    await expect(page.getByText("WhatsApp error codes: 131026")).toBeVisible();
    await expect(
      page.getByText("Component 1: Read", { exact: true }),
    ).toHaveCount(2);
    for (const details of await page.locator("details[open]").all()) {
      expect(
        await details.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true);
    }
    expect(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= window.innerWidth,
      ),
    ).toBe(true);
    await page.screenshot({
      path: test
        .info()
        .outputPath(`whatsapp-message-metadata-${String(width)}.png`),
      fullPage: true,
    });
    expect(errors).toEqual([]);
  });
}
