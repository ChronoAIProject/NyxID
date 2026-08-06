import { describe, expect, it } from "vitest";
import {
  connectionWebhookFieldsSchema,
  connectionWebhookFormSchema,
  connectionWebhookSecretResponseSchema,
} from "./connection-webhooks";

describe("connection webhook schemas", () => {
  it("parses the one-time secret response exactly as returned by the backend", () => {
    const parsed = connectionWebhookSecretResponseSchema.parse({
      client_id: "client-1",
      connection_webhook_url: "https://events.example.com/nyxid",
      connection_webhook_enabled: true,
      signing_secret: "nyx_whsec_once",
    });

    expect(parsed.signing_secret).toBe("nyx_whsec_once");
    expect(parsed.connection_webhook_enabled).toBe(true);
  });

  it("parses webhook fields on a developer app record", () => {
    expect(
      connectionWebhookFieldsSchema.parse({
        connection_webhook_url: null,
        connection_webhook_enabled: false,
      }),
    ).toEqual({
      connection_webhook_url: null,
      connection_webhook_enabled: false,
    });
  });

  it("accepts HTTPS endpoints and rejects other URL schemes", () => {
    expect(
      connectionWebhookFormSchema.safeParse({
        url: "https://events.example.com/nyxid",
      }).success,
    ).toBe(true);
    expect(
      connectionWebhookFormSchema.safeParse({
        url: "http://events.example.com/nyxid",
      }).success,
    ).toBe(false);
    expect(
      connectionWebhookFormSchema.safeParse({ url: "not-a-url" }).success,
    ).toBe(false);
  });
});
