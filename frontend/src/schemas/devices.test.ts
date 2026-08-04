import { describe, expect, it } from "vitest";
import {
  approveDeviceFormSchema,
  approveDeviceResponseSchema,
  formatDeviceUserCodeInput,
  maskIdentifier,
  normalizeDeviceUserCode,
  onboardDeviceFormSchema,
  onboardDeviceResponseSchema,
} from "./devices";

describe("normalizeDeviceUserCode", () => {
  it("accepts compact, dashed, spaced, and lowercase codes", () => {
    expect(normalizeDeviceUserCode("abcd efgh jklm")).toBe("ABCD-EFGH-JKLM");
    expect(normalizeDeviceUserCode("abcd-efgh-jklm")).toBe("ABCD-EFGH-JKLM");
    expect(normalizeDeviceUserCode("abcdefghjklm")).toBe("ABCD-EFGH-JKLM");
  });

  it("rejects ambiguous characters and wrong lengths", () => {
    expect(() => normalizeDeviceUserCode("ABCD-EFGH-IJKL")).toThrow();
    expect(() => normalizeDeviceUserCode("ABCD-EFGH-OJKL")).toThrow();
    expect(() => normalizeDeviceUserCode("ABCD-EFGH-JKL")).toThrow();
  });
});

describe("formatDeviceUserCodeInput", () => {
  it("uppercases, filters invalid characters, truncates, and inserts dashes", () => {
    expect(formatDeviceUserCodeInput("abcd efgh ijkl mnop")).toBe(
      "ABCD-EFGH-JKLM",
    );
  });

  it("keeps partial groups stable while typing", () => {
    expect(formatDeviceUserCodeInput("ab")).toBe("AB");
    expect(formatDeviceUserCodeInput("abcde")).toBe("ABCD-E");
    expect(formatDeviceUserCodeInput("abcdefgh")).toBe("ABCD-EFGH");
  });
});

describe("approveDeviceFormSchema", () => {
  it("normalizes the request payload", () => {
    const result = approveDeviceFormSchema.parse({
      user_code: "abcd efgh jklm",
      org_id: "550e8400-e29b-41d4-a716-446655440000",
      label: "  Hall camera  ",
    });

    expect(result).toEqual({
      user_code: "ABCD-EFGH-JKLM",
      org_id: "550e8400-e29b-41d4-a716-446655440000",
      label: "Hall camera",
    });
  });

  it("omits personal org and blank label", () => {
    const result = approveDeviceFormSchema.parse({
      user_code: "ABCD-EFGH-JKLM",
      org_id: null,
      label: "   ",
    });

    expect(result).toEqual({
      user_code: "ABCD-EFGH-JKLM",
      org_id: undefined,
      label: undefined,
    });
  });

  it("accepts optional default services", () => {
    expect(
      approveDeviceFormSchema.parse({
        user_code: "ABCD-EFGH-JKLM",
        org_id: null,
        label: "",
      }),
    ).not.toHaveProperty("default_services");

    expect(
      approveDeviceFormSchema.parse({
        user_code: "ABCD-EFGH-JKLM",
        org_id: null,
        label: "",
        default_services: [],
      }),
    ).toMatchObject({ default_services: [] });
  });

  it("rejects non-string default services", () => {
    const result = approveDeviceFormSchema.safeParse({
      user_code: "ABCD-EFGH-JKLM",
      org_id: null,
      label: "",
      default_services: ["llm-openai", 123],
    });

    expect(result.success).toBe(false);
  });

  it("rejects overlong labels", () => {
    const result = approveDeviceFormSchema.safeParse({
      user_code: "ABCD-EFGH-JKLM",
      org_id: null,
      label: "x".repeat(201),
    });

    expect(result.success).toBe(false);
  });
});

describe("approveDeviceResponseSchema", () => {
  it("parses the backend approval response", () => {
    const result = approveDeviceResponseSchema.safeParse({
      device_label: "Hall camera",
      hw_id: "esp32-aabbcc",
      api_key_id: "7ef9c1a4-8df9-43af-9f92-98a6c9a7f45d",
      node_id: "4df27e8f-8cb5-47b7-8d29-e6529f2c1c40",
      owner_user_id: "131ff391-d7d6-49ed-a2ef-c94b9ed95d40",
      org_id: null,
    });

    expect(result.success).toBe(true);
  });
});

describe("onboardDeviceFormSchema", () => {
  it("normalizes the onboard request payload", () => {
    const result = onboardDeviceFormSchema.parse({
      org_id: null,
      label: "  Kitchen Camera  ",
      wifi_ssid: "  MyHomeNetwork  ",
      wifi_password: "hunter22",
      default_services: ["llm-openai"],
    });

    expect(result).toEqual({
      org_id: undefined,
      label: "Kitchen Camera",
      wifi_ssid: "MyHomeNetwork",
      wifi_password: "hunter22",
      default_services: ["llm-openai"],
    });
  });

  it("accepts undefined and empty default services", () => {
    expect(
      onboardDeviceFormSchema.parse({
        org_id: null,
        label: "Kitchen Camera",
        wifi_ssid: "MyHomeNetwork",
        wifi_password: "hunter22",
      }),
    ).not.toHaveProperty("default_services");

    expect(
      onboardDeviceFormSchema.parse({
        org_id: null,
        label: "Kitchen Camera",
        wifi_ssid: "MyHomeNetwork",
        wifi_password: "hunter22",
        default_services: [],
      }),
    ).toMatchObject({ default_services: [] });
  });

  it("rejects invalid WiFi fields and non-string default services", () => {
    expect(
      onboardDeviceFormSchema.safeParse({
        org_id: null,
        label: "Kitchen Camera",
        wifi_ssid: "x".repeat(33),
        wifi_password: "hunter22",
      }).success,
    ).toBe(false);

    expect(
      onboardDeviceFormSchema.safeParse({
        org_id: null,
        label: "Kitchen Camera",
        wifi_ssid: "MyHomeNetwork",
        wifi_password: "short",
      }).success,
    ).toBe(false);

    expect(
      onboardDeviceFormSchema.safeParse({
        org_id: null,
        label: "Kitchen Camera",
        wifi_ssid: "MyHomeNetwork",
        wifi_password: "hunter22",
        default_services: ["llm-openai", 123],
      }).success,
    ).toBe(false);
  });
});

describe("onboardDeviceResponseSchema", () => {
  it("parses the backend onboard response", () => {
    const result = onboardDeviceResponseSchema.safeParse({
      qr_payload: "nyxprov://full?ssid=Home",
      node_id: "4df27e8f-8cb5-47b7-8d29-e6529f2c1c40",
      api_key_id: "7ef9c1a4-8df9-43af-9f92-98a6c9a7f45d",
      label: "Kitchen Camera",
    });

    expect(result.success).toBe(true);
  });
});

describe("maskIdentifier", () => {
  it("shortens long identifiers without changing short identifiers", () => {
    expect(maskIdentifier("7ef9c1a4-8df9-43af-9f92-98a6c9a7f45d")).toBe(
      "7ef9c1a4...",
    );
    expect(maskIdentifier("short")).toBe("short");
  });
});
