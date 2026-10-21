// @vitest-environment node
import { expect, it } from "vitest";
import { createHash } from "node:crypto";
// @ts-expect-error Build-only module.
import {
  retainedDisabledFeatureCode,
  verifyReleaseConfiguration,
} from "../scripts/public-visibility-contract.mjs";
const hash = (value: string) =>
  createHash("sha256").update(value).digest("hex");
it.each(['"', "'", "`"])(
  "detects disabled controls using %s string literals with independent switches",
  (quote) => {
    const names = ["newsletter", "contact-chat", "footer-status"];
    const code = names
      .map((name) => `{"data-website-feature":${quote}${name}${quote}}`)
      .join(";");
    expect(
      retainedDisabledFeatureCode(code, {
        newsletter: false,
        contactChat: false,
        footerStatus: false,
      }),
    ).toEqual(names);
    for (const [enabledName, enabledKey] of [
      ["newsletter", "newsletter"],
      ["contact-chat", "contactChat"],
      ["footer-status", "footerStatus"],
    ]) {
      expect(
        retainedDisabledFeatureCode(code, {
          newsletter: false,
          contactChat: false,
          footerStatus: false,
          [enabledKey]: true,
        }),
      ).toEqual(names.filter((name) => name !== enabledName));
    }
  },
);
it("requires a complete static marker rather than a prefix or interpolated template", () => {
  expect(
    retainedDisabledFeatureCode(
      '{"data-website-feature":`newsletter${suffix}`} ; {"data-website-feature":"contact-chat-preview"}',
      { newsletter: false, contactChat: false, footerStatus: false },
    ),
  ).toEqual([]);
});
it("still rejects retained newsletter/chat code when Status alone is enabled", () => {
  expect(
    retainedDisabledFeatureCode(
      `{"data-website-feature":"newsletter"};{"data-website-feature":"contact-chat"};{"data-website-feature":"footer-status"}`,
      { newsletter: false, contactChat: false, footerStatus: true },
    ),
  ).toEqual(["newsletter", "contact-chat"]);
});
it("rejects configuration or content revision drift", () => {
  const release = {
    configuration: { newsletter: { endpoint: "/api/first" } },
    visibility: { newsletter: true },
    pages: [],
    configRevision: "",
    contentRevision: "",
  };
  release.configRevision = hash(JSON.stringify(release.configuration));
  release.contentRevision = hash(
    JSON.stringify({
      pages: release.pages,
      visibility: release.visibility,
      configRevision: release.configRevision,
    }),
  );
  expect(() => verifyReleaseConfiguration(release)).not.toThrow();
  expect(() =>
    verifyReleaseConfiguration({
      ...release,
      configuration: { newsletter: { endpoint: "/api/second" } },
    }),
  ).toThrow(/revision/);
  expect(() => verifyReleaseConfiguration({ ...release, pages: [{}] })).toThrow(
    /revision/,
  );
});
