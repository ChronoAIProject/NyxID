import assert from "node:assert/strict";
import test from "node:test";
import { creditUnits, formatExactCredits } from "./credits";

test("credit display matches six-decimal half-up rounding without losing exact arithmetic", () => {
  for (const [raw, shown] of [
    ["248.250000000001", "248.25"], ["1.875000000002", "1.875"],
    ["1.234567499999", "1.234567"], ["1.234567500000", "1.234568"],
    ["-1.234567500000", "-1.234568"], ["0.0000015", "0.000002"],
    ["999.999999500000", "1,000"], ["-999.999999500000", "-1,000"],
    ["9007199254740993.123456500001", "9,007,199,254,740,993.123457"],
    ["9999999999999999999999.999999999999", "10,000,000,000,000,000,000,000"],
    ["0", "0"], ["-0.000000000000", "0"],
  ]) assert.equal(formatExactCredits(raw!, "en-US"), shown);
  assert.equal(creditUnits("248.250000000001"), 248_250_000_000_001n);
});

test("sub-micro nonzero credits retain their sign and are never displayed as zero", () => {
  for (const value of ["0.000000000001", "0.000000499999", "0.000000999999"]) {
    assert.equal(formatExactCredits(value, "en-US"), "<0.000001");
    assert.equal(formatExactCredits(`-${value}`, "en-US"), ">-0.000001");
  }
  assert.equal(formatExactCredits("0.000001", "en-US"), "0.000001");
  assert.equal(formatExactCredits("-0.000001", "en-US"), "-0.000001");
});

test("whole credits and fractional separators follow the locale using BigInt", () => {
  assert.equal(formatExactCredits("1234567.1234565", "de-DE"), "1.234.567,123457");
  assert.equal(formatExactCredits("1234567.1234565", "en-IN"), "12,34,567.123457");
  assert.equal(formatExactCredits("0.000000000001", "de-DE"), "<0,000001");
  assert.equal(formatExactCredits("-0.000000000001", "de-DE"), ">-0,000001");
  assert.equal(formatExactCredits("1234.125", "es-ES"), "1234,125");
  assert.equal(formatExactCredits("12345.125", "es-ES"), "12.345,125");
});

test("Hermes-compatible Intl without parts or BigInt support still formats huge money exactly", (t) => {
  const NativeNumberFormat = Intl.NumberFormat;
  t.mock.method(Intl, "NumberFormat", function (locale?: string, options?: Intl.NumberFormatOptions) {
    const formatter = new NativeNumberFormat(locale, options);
    return { format(value: number | bigint) {
      assert.equal(typeof value, "number", "only fixed integer locale probes may enter native Intl");
      assert.ok(Number.isSafeInteger(value));
      return formatter.format(value);
    } };
  } as unknown as typeof Intl.NumberFormat);
  assert.equal(formatExactCredits("9007199254740993.123456500001", "de-DE"), "9.007.199.254.740.993,123457");
  assert.equal(formatExactCredits("1234567.0000015", "en-IN"), "12,34,567.000002");
});

test("removing Intl.NumberFormat.prototype.formatToParts leaves the display unchanged", (t) => {
  const cases = ["248.250000000001", "-0.000000000001", "999.9999995", "9007199254740993.1234565"];
  const before = cases.map((value) => formatExactCredits(value, "de-DE"));
  const descriptor = Object.getOwnPropertyDescriptor(Intl.NumberFormat.prototype, "formatToParts")!;
  Object.defineProperty(Intl.NumberFormat.prototype, "formatToParts", { ...descriptor, value: undefined });
  t.after(() => Object.defineProperty(Intl.NumberFormat.prototype, "formatToParts", descriptor));
  assert.deepEqual(cases.map((value) => formatExactCredits(value, "de-DE")), before);
});
