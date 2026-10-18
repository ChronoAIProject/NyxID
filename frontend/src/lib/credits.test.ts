import { describe, expect, it } from "vitest";
import { formatCreditUnits, formatMicrocredits } from "./credits";

describe("credit formatters", () => {
  it("keeps all six fractional digits without Number conversion", () => {
    expect(
      formatMicrocredits("100250001", { suffix: false, locale: "en-US" }),
    ).toBe("100.250001");
    expect(formatMicrocredits("5", { suffix: false, locale: "en-US" })).toBe(
      "0.000005",
    );
  });

  it("formats values at the signed i64 boundary exactly", () => {
    expect(
      formatMicrocredits("9223372036854775807", {
        suffix: false,
        locale: "en-US",
      }),
    ).toBe("9,223,372,036,854.775807");
    expect(
      formatCreditUnits("9223372036854775807", {
        suffix: false,
        locale: "en-US",
      }),
    ).toBe("9,223,372,036,854,775,807");
  });

  it("localizes fractional digits and retains locale signs without losing precision", () => {
    expect(formatMicrocredits("1234500", { locale: "ar-EG" })).toBe(
      "١٫٢٣٤٥ credits",
    );
    expect(formatMicrocredits("-1234500", { locale: "ar-EG" })).toBe(
      "؜-١٫٢٣٤٥ credits",
    );
    expect(formatMicrocredits("-1", { locale: "ar-EG", suffix: false })).toBe(
      "؜-٠٫٠٠٠٠٠١",
    );
    expect(
      formatMicrocredits("922337203685477580700000001", { locale: "ar-EG" }),
    ).toBe("٩٢٢٬٣٣٧٬٢٠٣٬٦٨٥٬٤٧٧٬٥٨٠٬٧٠٠٫٠٠٠٠٠١ credits");
    expect(formatMicrocredits("-1", { locale: "en-US" })).toBe(
      "-0.000001 credits",
    );
  });

  it("uses the locale's grouping and decimal separator", () => {
    expect(
      formatMicrocredits("1234500000", { suffix: false, locale: "de-DE" }),
    ).toBe("1.234,5");
  });
});
