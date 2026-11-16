import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";
import { z } from "zod";
import { updateProfileSchema } from "./accountSchema";

const expoRequire = createRequire(
  createRequire(import.meta.url).resolve("expo/package.json"),
);
// Resolve Expo's actual Winter URL dependency, not an independently installed polyfill.
const expoURL = expoRequire("whatwg-url-minimum").URL as typeof URL;
const marks = [
  0x200b, 0x200c, 0x200d, 0x200e, 0x200f, 0x202a, 0x202b, 0x202c, 0x202d,
  0x202e, 0x2060, 0x2066, 0x2067, 0x2068, 0x2069, 0xad, 0xfeff,
];
for (const [name, implementation] of [
  ["Node", URL],
  ["Expo Winter", expoURL],
] as const) {
  test(`${name} avatar schema saves lowercase schemes and measures the normalized UTF-8 value`, () => {
    const original = globalThis.URL;
    globalThis.URL = implementation;
    try {
      for (const scheme of ["HTTPS", "Http", "hTtPs"])
        for (const separator of ["://", "：／／"]) {
          const input = `${scheme}${separator}avatars.githubusercontent.com/u/1234?s=420&v=4`;
          assert.equal(
            updateProfileSchema.parse({ avatar_url: input }).avatar_url,
            `${scheme.toLowerCase()}://avatars.githubusercontent.com/u/1234?s=420&v=4`,
          );
        }
      const prefix = "https://example.com/";
      const boundary = prefix + "a".repeat(2048 - Buffer.byteLength(prefix));
      const marks = "\u200e\u200f\ufeff".repeat(500);
      assert.equal(
        updateProfileSchema.parse({ avatar_url: marks + boundary + marks })
          .avatar_url,
        boundary,
      );
      const multiByte = prefix + "界".repeat(676);
      assert.ok(Buffer.byteLength(multiByte) <= 2048);
      assert.equal(
        updateProfileSchema.parse({ avatar_url: marks + multiByte }).avatar_url,
        multiByte,
      );
      for (const input of [boundary + "a", marks + multiByte + "界" + marks]) {
        const parsed = updateProfileSchema.safeParse({ avatar_url: input });
        assert.equal(parsed.success, false);
        if (!parsed.success)
          assert.equal(
            parsed.error.issues[0]!.message,
            "Avatar URL is too long (maximum 2048 bytes).",
          );
      }
    } finally {
      globalThis.URL = original;
    }
  });
  test(`${name} avatar schema saves normalized GitHub avatar URLs with pasted format marks and IME punctuation`, () => {
    const original = globalThis.URL;
    globalThis.URL = implementation;
    try {
      for (const query of ["?v=4", "?s=420&v=4"]) {
        const url = `https://avatars.githubusercontent.com/u/1234${query}`;
        for (const mark of ["", ...marks.map((c) => String.fromCodePoint(c))]) {
          for (const input of [
            `${mark}${url}${mark}`,
            `${mark}${url.replace(":", "：")}${mark}`,
            ` \n${url}\t `,
          ]) {
            assert.equal(
              updateProfileSchema.parse({ avatar_url: input }).avatar_url,
              url,
            );
          }
        }
      }
    } finally {
      globalThis.URL = original;
    }
  });
  test(`${name} avatar schema rejects unsafe schemes, incomplete URLs and oversized input with rule-specific copy`, () => {
    const original = globalThis.URL;
    globalThis.URL = implementation;
    try {
      for (const [input, rule] of [
        ["javascript:alert(1)", "must start"],
        ["data:image/png;base64,abc", "must start"],
        ["ftp://example.com/a", "must start"],
        ["garbage", "must start"],
        ["https://", "complete web address"],
        ["https://example.com/" + "界".repeat(700), "2048 bytes"],
      ]) {
        const parsed = updateProfileSchema.safeParse({ avatar_url: input });
        assert.equal(parsed.success, false, input);
        if (!parsed.success)
          assert.match(parsed.error.issues[0]!.message, new RegExp(rule!));
      }
    } finally {
      globalThis.URL = original;
    }
  });
}

function utf8Length(value: string): number {
  return Buffer.byteLength(value, "utf8");
}
// Main's original validation chain is the compatibility oracle.
const originMainAvatarUrl = z
  .string()
  .trim()
  .url("Enter a valid avatar URL")
  .refine(
    (v) => /^https?:\/\//.test(v),
    "Avatar URL must use https:// or http://",
  )
  .refine((v) => utf8Length(v) <= 2048, "Avatar URL is too long");

type AvatarRow = [
  input: string,
  mainAccepts: boolean,
  branchAccepts: boolean,
  storedValue: string | null,
  edgeMarksOnly: boolean,
];
const expectedTables: Record<string, AvatarRow[]> = {
  Node: [
    [
      "https://example.com/my avatar.png",
      true,
      true,
      "https://example.com/my avatar.png",
      false,
    ],
    ["https://exam\u200dple.com/a", false, false, null, false],
    [
      "https://example.com/my\u200cavatar.png",
      true,
      true,
      "https://example.com/my\u200cavatar.png",
      false,
    ],
    [
      "\u200ehttps://example.com/a",
      false,
      true,
      "https://example.com/a",
      false,
    ],
    ["https://example.com/a\u200b", true, true, "https://example.com/a", true],
    ["HTTPS://example.com/a", false, true, "https://example.com/a", false],
    ["https：//example.com/a", false, true, "https://example.com/a", false],
    [
      "https://avatars.githubusercontent.com/u/1234?s=420&v=4",
      true,
      true,
      "https://avatars.githubusercontent.com/u/1234?s=420&v=4",
      false,
    ],
    ["https://exam ple.com/a", false, false, null, false],
    ["javascript:alert(1)", false, false, null, false],
  ],
  "Expo Winter": [
    [
      "https://example.com/my avatar.png",
      true,
      true,
      "https://example.com/my avatar.png",
      false,
    ],
    [
      "https://exam\u200dple.com/a",
      true,
      true,
      "https://exam\u200dple.com/a",
      false,
    ],
    [
      "https://example.com/my\u200cavatar.png",
      true,
      true,
      "https://example.com/my\u200cavatar.png",
      false,
    ],
    [
      "\u200ehttps://example.com/a",
      false,
      true,
      "https://example.com/a",
      false,
    ],
    ["https://example.com/a\u200b", true, true, "https://example.com/a", true],
    ["HTTPS://example.com/a", false, true, "https://example.com/a", false],
    ["https：//example.com/a", false, true, "https://example.com/a", false],
    [
      "https://avatars.githubusercontent.com/u/1234?s=420&v=4",
      true,
      true,
      "https://avatars.githubusercontent.com/u/1234?s=420&v=4",
      false,
    ],
    ["https://exam ple.com/a", false, false, null, false],
    ["javascript:alert(1)", false, false, null, false],
  ],
};
const trailingMarks = /[\s\p{Cf}]+$/u;
const printable = (value: string | null) =>
  value === null
    ? "—"
    : value.replace(
        /[\p{Cf}\p{Cc}]/gu,
        (c) => `\\u${c.codePointAt(0)!.toString(16).padStart(4, "0")}`,
      );

for (const [name, implementation] of [
  ["Node", URL],
  ["Expo Winter", expoURL],
] as const) {
  test(`${name} avatar compatibility table preserves every origin/main accepted resource`, (t) => {
    const original = globalThis.URL;
    globalThis.URL = implementation;
    try {
      const actual: AvatarRow[] = expectedTables[name]!.map(([input]) => {
        const main = originMainAvatarUrl.safeParse(input);
        const branch = updateProfileSchema.safeParse({ avatar_url: input });
        const stored = branch.success ? branch.data.avatar_url! : null;
        if (main.success) {
          assert.ok(branch.success, `main accepted ${JSON.stringify(input)}`);
          assert.ok(
            stored === main.data ||
              stored === main.data.replace(trailingMarks, ""),
          );
        }
        return [
          input,
          main.success,
          branch.success,
          stored,
          main.success &&
            stored !== main.data &&
            stored === main.data.replace(trailingMarks, ""),
        ];
      });
      assert.deepEqual(actual, expectedTables[name]);
      for (const [input, mainAccepts, , stored, edgeMarksOnly] of actual) {
        if (mainAccepts) {
          const mainValue = originMainAvatarUrl.parse(input);
          assert.equal(
            stored,
            edgeMarksOnly ? mainValue.replace(trailingMarks, "") : mainValue,
          );
        }
      }
      for (const line of [
        name,
        "| Input | Main accepts | Branch accepts | Branch stored value | Edge marks only |",
        "| --- | --- | --- | --- | --- |",
        ...actual.map(
          (row) =>
            `| ${printable(row[0])} | ${row[1]} | ${row[2]} | ${printable(row[3])} | ${row[4]} |`,
        ),
      ])
        t.diagnostic(line);
    } finally {
      globalThis.URL = original;
    }
  });

  test(`${name} avatar prefix/body/suffix sweep is a superset of origin/main with ordered rejection copy`, () => {
    const original = globalThis.URL;
    globalThis.URL = implementation;
    try {
      const prefix = "https://example.com/";
      const boundary = prefix + "a".repeat(2048 - utf8Length(prefix));
      const bodies = [
        ...expectedTables[name]!.map((row) => row[0]),
        "https://example.com/my\navatar.png",
        "https://example.com/my\tavatar.png",
        "https://example.com/my\u200bavatar.png",
        "https:／／example.com/a",
        "https://",
        boundary,
        boundary + "a",
      ];
      for (const start of ["", " ", "\u200e", "\ufeff", " \u200b"])
        for (const body of bodies)
          for (const end of ["", " ", "\u200b", "\u200b ", "\t\u200f"]) {
            const input = start + body + end;
            const main = originMainAvatarUrl.safeParse(input);
            const branch = updateProfileSchema.safeParse({ avatar_url: input });
            if (main.success) {
              assert.ok(
                branch.success,
                `main accepted ${JSON.stringify(input)}`,
              );
              const stored = branch.data.avatar_url;
              assert.ok(
                stored === main.data ||
                  stored === main.data.replace(trailingMarks, ""),
                `interior changed for ${JSON.stringify(input)}`,
              );
            }
          }
      assert.equal(updateProfileSchema.parse({ avatar_url: "https://[::1]/a" }).avatar_url, "https://[::1]/a");
      for (const [input, message] of [
        [
          "https://exam ple.com/a",
          "Remove spaces or hidden characters from inside the avatar URL.",
        ],
        [
          "javascript:alert(1)",
          "Avatar URL must start with https:// or http://.",
        ],
        [boundary + "a", "Avatar URL is too long (maximum 2048 bytes)."],
        [
          "https://[::\u200b1]/a",
          "Remove spaces or hidden characters from inside the avatar URL.",
        ],
        [
          "javascript:" + "a".repeat(2050),
          "Avatar URL must start with https:// or http://.",
        ],
        [boundary + " a", "Avatar URL is too long (maximum 2048 bytes)."],
        ["https://", "Avatar URL isn't a complete web address."],
      ]) {
        const parsed = updateProfileSchema.safeParse({ avatar_url: input });
        assert.equal(parsed.success, false, input);
        if (!parsed.success)
          assert.equal(parsed.error.issues[0]!.message, message);
      }
    } finally {
      globalThis.URL = original;
    }
  });
}
