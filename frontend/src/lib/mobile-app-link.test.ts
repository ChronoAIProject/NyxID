import { describe, expect, it } from "vitest";
import { resolveMobileAppLink } from "./mobile-app-link";

describe("mobile app download URL", () => {
  it("accepts a provisioned HTTPS URL", () => {
    expect(resolveMobileAppLink(" https://download.example/app?platform=auto "))
      .toBe("https://download.example/app?platform=auto");
  });

  it.each([undefined, "", " ", "not-a-url", "https://", "javascript:alert(1)", "nyxid://app", "http://download.example"])(
    "keeps the existing OneLink fallback for %s", (value) => {
      expect(resolveMobileAppLink(value)).toBe("https://nyxid.onelink.me/REzJ/dql9w8fx");
    },
  );
});
