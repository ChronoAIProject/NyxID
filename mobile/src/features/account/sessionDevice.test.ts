import assert from "node:assert/strict";
import test from "node:test";
import { sessionDeviceLabel } from "./sessionDevice";

test("session rows name the browser/app and device instead of the raw user agent", () => {
  const cases: [string | null | undefined, string][] = [
    [
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
      "Safari on iPhone",
    ],
    [
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36",
      "Chrome on Mac",
    ],
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Edg/130.0",
      "Edge on Windows",
    ],
    [
      "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0",
      "Firefox on Linux",
    ],
    [
      "NyxIDMobile/1.0.4 CFNetwork/1568 Darwin/24.0.0 iPhone",
      "NyxID app on iPhone",
    ],
    ["NyxID mobile", "NyxID app"],
    ["nyxid-cli/0.77.0", "NyxID CLI"],
    ["", "Unknown device"],
    [null, "Unknown device"],
    ["curl/8.7.1", "curl/8.7.1"],
  ];
  for (const [ua, label] of cases)
    assert.equal(sessionDeviceLabel(ua), label, String(ua));
});
