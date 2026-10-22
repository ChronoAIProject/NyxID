import { expect, it } from "vitest";
import { resolveWebsiteRelease } from "./release-config";
const complete = {
  VITE_WEBSITE_NEWSLETTER_ENDPOINT: "/api/public/newsletter",
  VITE_WEBSITE_NEWSLETTER_PRIVACY_URL: "https://nyx.chrono-ai.fun/privacy",
  VITE_WEBSITE_NEWSLETTER_UNSUBSCRIBE_URL:
    "https://status.example.test/unsubscribe",
  VITE_WEBSITE_NEWSLETTER_ACCEPTANCE: "review/newsletter-accepted",
  VITE_WEBSITE_CONTACT_CHAT_ENDPOINT: "/api/public/chat",
  VITE_WEBSITE_CONTACT_CHAT_ACCEPTANCE: "review/chat-accepted",
  VITE_WEBSITE_STATUS_URL: "https://status.example.test/",
  VITE_WEBSITE_STATUS_ACCEPTANCE: "review/status-accepted",
};
it("defaults every optional feature and solution to off", () => {
  expect(resolveWebsiteRelease({})).toEqual({
    blogMenu: false,
    newsletter: null,
    contactChat: null,
    footerStatus: null,
    solutions: {},
  });
  expect(
    resolveWebsiteRelease({
      ...complete,
      VITE_WEBSITE_NEWSLETTER_ENABLED: "false",
    }).newsletter,
  ).toBeNull();
});
it.each(Array.from({ length: 8 }, (_, i) => i))(
  "keeps all three switches independent (%i)",
  (mask) => {
    const release = resolveWebsiteRelease({
      ...complete,
      VITE_WEBSITE_NEWSLETTER_ENABLED: String(Boolean(mask & 1)),
      VITE_WEBSITE_CONTACT_CHAT_ENABLED: String(Boolean(mask & 2)),
      VITE_WEBSITE_STATUS_ENABLED: String(Boolean(mask & 4)),
    });
    expect(Boolean(release.newsletter)).toBe(Boolean(mask & 1));
    expect(Boolean(release.contactChat)).toBe(Boolean(mask & 2));
    expect(Boolean(release.footerStatus)).toBe(Boolean(mask & 4));
    expect(release.solutions).toEqual({});
  },
);
it.each(["NEWSLETTER", "CONTACT_CHAT", "STATUS"])(
  "refuses enabled %s without a complete accepted adapter",
  (feature) => {
    expect(() =>
      resolveWebsiteRelease({ [`VITE_WEBSITE_${feature}_ENABLED`]: "true" }),
    ).toThrow(/configuration/);
    expect(() =>
      resolveWebsiteRelease({
        ...complete,
        [`VITE_WEBSITE_${feature}_ENABLED`]: "true",
        [`VITE_WEBSITE_${feature}_ACCEPTANCE`]: "",
      }),
    ).toThrow(/configuration/);
  },
);
it("rejects credential-bearing endpoints and fake status destinations", () => {
  expect(() =>
    resolveWebsiteRelease({
      ...complete,
      VITE_WEBSITE_CONTACT_CHAT_ENABLED: "true",
      VITE_WEBSITE_CONTACT_CHAT_ENDPOINT:
        "https://other.test/chat?token=secret",
    }),
  ).toThrow();
  for (const url of [
    "/",
    "https://nyx.chrono-ai.fun/",
    "https://nyx.chrono-ai.fun/?",
    "https://nyx.chrono-ai.fun/#",
    "https://nyx.chrono-ai.fun/zh",
    "https://nyx.chrono-ai.fun/zh/",
    "https://nyx.chrono-ai.fun/zh//",
    "https://nyx.chrono-ai.fun/%7A%68",
    "https://nyx.chrono-ai.fun/%7A%68%2f%2f",
    "https://nyx.chrono-ai.fun/zh%2f..%2fzh",
    "https://nyx.chrono-ai.fun/%invalid",
    "https://status.example.test/health",
    "javascript:alert(1)",
  ])
    expect(() =>
      resolveWebsiteRelease({
        ...complete,
        VITE_WEBSITE_STATUS_ENABLED: "true",
        VITE_WEBSITE_STATUS_URL: url,
      }),
    ).toThrow();
});
it("allows the root of an independent public status site", () => {
  expect(
    resolveWebsiteRelease({
      ...complete,
      VITE_WEBSITE_STATUS_ENABLED: "true",
    }).footerStatus,
  ).toBe("https://status.example.test/");
});
it("enables only an individually accepted solution and rejects unsafe targets", () => {
  const release = resolveWebsiteRelease({
    VITE_WEBSITE_SOLUTION_ACTIVATIONS: JSON.stringify({
      "help-desk": {
        target: "/assistant?setup=help-desk",
        acceptance: "review/help-desk",
      },
    }),
  });
  expect(release.solutions).toEqual({
    "help-desk": "/assistant?setup=help-desk",
  });
  for (const target of [
    "//evil.test",
    "javascript:alert(1)",
    "/api/run",
    "/register?return_to=//evil.test",
  ])
    expect(() =>
      resolveWebsiteRelease({
        VITE_WEBSITE_SOLUTION_ACTIVATIONS: JSON.stringify({
          "help-desk": { target, acceptance: "review/help-desk" },
        }),
      }),
    ).toThrow();
});

it("hides only the Blog menu by default and restores it independently", () => {
  const release = resolveWebsiteRelease({
    VITE_WEBSITE_BLOG_MENU_ENABLED: "true",
  });
  expect(release.blogMenu).toBe(true);
  expect(release.newsletter).toBeNull();
  expect(release.contactChat).toBeNull();
  expect(release.footerStatus).toBeNull();
});
