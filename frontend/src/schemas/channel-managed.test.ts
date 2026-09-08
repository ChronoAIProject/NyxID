import { describe, expect, it } from "vitest";
import {
  managedCompleteSchema,
  managedBootstrapSchema,
} from "./channel-managed";
import {
  platformCredentialFieldSchema,
  platformCredentialFormSchema,
} from "./admin-platform-credentials";
import { parseEmbeddedSignupEvent } from "@/lib/meta-embedded-signup";

describe("managed onboarding boundaries", () => {
  it("accepts unavailable bootstrap and WABA-only completion", () => {
    expect(managedBootstrapSchema.parse({ available: false }).available).toBe(
      false,
    );
    expect(
      managedCompleteSchema.safeParse({
        code: "one-use",
        waba_id: "123",
        label: "Support",
      }).success,
    ).toBe(true);
    for (const input of [
      { code: "" },
      { phone_number_id: "../token" },
      { waba_id: "x" },
      { label: " " },
    ])
      expect(
        managedCompleteSchema.safeParse({
          code: "one-use",
          waba_id: "123",
          label: "Support",
          ...input,
        }).success,
      ).toBe(false);
  });
  it("accepts only the exact Facebook origin and supported events", () => {
    const data = {
      type: "WA_EMBEDDED_SIGNUP",
      event: "FINISH",
      data: { waba_id: "123", phone_number_id: "456" },
    };
    for (const origin of [
      "https://facebook.com",
      "https://www.facebook.com.attacker.test",
      "http://www.facebook.com",
      "null",
    ])
      expect(
        parseEmbeddedSignupEvent(new MessageEvent("message", { origin, data })),
      ).toBeNull();
    expect(
      parseEmbeddedSignupEvent(
        new MessageEvent("message", {
          origin: "https://www.facebook.com",
          data: JSON.stringify(data),
        }),
      )?.data.phone_number_id,
    ).toBe("456");
    for (const event of [
      "FINISH_ONLY_WABA",
      "FINISH_WHATSAPP_BUSINESS_APP_ONBOARDING",
      "CANCEL",
      "ERROR",
    ])
      expect(
        parseEmbeddedSignupEvent(
          new MessageEvent("message", {
            origin: "https://www.facebook.com",
            data: { ...data, event },
          }),
        )?.event,
      ).toBe(event);
    expect(
      parseEmbeddedSignupEvent(
        new MessageEvent("message", {
          origin: "https://www.facebook.com",
          data: "broken JSON",
        }),
      ),
    ).toBeNull();
  });
  it("allows explicit clears while rejecting secret values in responses", () => {
    expect(
      platformCredentialFormSchema.parse({ fields: { app_secret: null } })
        .fields.app_secret,
    ).toBeNull();
    const field = {
      name: "app_secret",
      label: "Secret",
      secret: true,
      help: "",
      required: true,
      numeric: false,
      configured: true,
    };
    expect(platformCredentialFieldSchema.safeParse(field).success).toBe(true);
    expect(
      platformCredentialFieldSchema.safeParse({ ...field, value: "leaked" })
        .success,
    ).toBe(false);
  });
});
