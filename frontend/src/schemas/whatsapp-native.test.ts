import { describe, expect, it } from "vitest";
import { whatsappNativeSchema } from "./whatsapp-native";

describe("native WhatsApp identity", () => {
  const valid = { meta_app_id: "111", waba_id: "222", dedicated_app: true };
  it("keeps the app and WABA separate", () => {
    expect(whatsappNativeSchema.parse(valid)).toEqual(valid);
    expect(whatsappNativeSchema.parse({ ...valid, meta_app_id: " 111 " }).meta_app_id).toBe("111");
  });
  it("requires dedicated-app confirmation and numeric IDs", () => {
    for (const patch of [{ dedicated_app: false }, { meta_app_id: "" }, { waba_id: "app/123" }, { meta_app_id: "1".repeat(33) }]) {
      expect(whatsappNativeSchema.safeParse({ ...valid, ...patch }).success).toBe(false);
    }
  });
});
