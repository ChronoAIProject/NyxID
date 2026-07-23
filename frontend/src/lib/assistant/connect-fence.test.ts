import { describe, expect, it } from "vitest";
import {
  connectMarkerToBlock,
  hasConnectMarker,
  renderableText,
  splitConnectMarkers,
} from "./connect-fence";

function marker(slug: string, reason = "read your merged PRs"): string {
  return [
    "```nyxid:connect",
    JSON.stringify({ catalog_slug: slug, reason }),
    "```",
  ].join("\n");
}

describe("splitConnectMarkers", () => {
  it("returns a single text segment when there is no marker", () => {
    expect(splitConnectMarkers("Just prose.")).toEqual([
      { kind: "text", text: "Just prose." },
    ]);
  });

  it("splits prose around a marker, preserving order", () => {
    const source = [
      "I need two things first.",
      marker("api-github"),
      "Then I'll summarise them.",
    ].join("\n");

    const segments = splitConnectMarkers(source);

    expect(segments.map((segment) => segment.kind)).toEqual([
      "text",
      "connect",
      "text",
    ]);
    expect(segments[1]).toMatchObject({
      kind: "connect",
      marker: { catalogSlug: "api-github", reason: "read your merged PRs" },
    });
  });

  it("emits every marker in one message (all gaps at once)", () => {
    const source = [
      "Two connections are missing:",
      marker("api-github"),
      marker("api-lark-bot"),
    ].join("\n");

    const connects = splitConnectMarkers(source).filter(
      (segment) => segment.kind === "connect",
    );

    expect(connects).toHaveLength(2);
  });

  it("accepts the `slug` alias and normalises case", () => {
    const source = [
      "```nyxid:connect",
      JSON.stringify({ slug: "API-GitHub" }),
      "```",
    ].join("\n");

    expect(splitConnectMarkers(source)[0]).toMatchObject({
      kind: "connect",
      marker: { catalogSlug: "api-github" },
    });
  });

  describe("does not turn quoted content into a live card", () => {
    it("ignores a marker nested inside an ordinary code fence", () => {
      // The injection path Ean's splitter is open to: anything that can get a
      // fenced block into the transcript (a tool result, a pasted log, the
      // model quoting its own instructions) could otherwise mint a card.
      const source = [
        "Here's how the encoding looks:",
        "````markdown",
        "```nyxid:connect",
        JSON.stringify({ catalog_slug: "api-github" }),
        "```",
        "````",
      ].join("\n");

      expect(
        splitConnectMarkers(source).every((segment) => segment.kind === "text"),
      ).toBe(true);
      expect(hasConnectMarker(source)).toBe(false);
    });

    it("ignores a marker inside a plain ``` block", () => {
      const source = [
        "```",
        "```nyxid:connect",
        JSON.stringify({ catalog_slug: "api-github" }),
        "```",
      ].join("\n");

      expect(hasConnectMarker(source)).toBe(false);
    });

    it("still parses a real marker that follows a closed code fence", () => {
      const source = [
        "```ts",
        "const x = 1;",
        "```",
        marker("api-github"),
      ].join("\n");

      expect(hasConnectMarker(source)).toBe(true);
    });
  });

  describe("malformed markers degrade to literal text", () => {
    it("keeps unparseable JSON as text", () => {
      const source = ["```nyxid:connect", "{not json", "```"].join("\n");

      const segments = splitConnectMarkers(source);

      expect(segments.every((segment) => segment.kind === "text")).toBe(true);
    });

    it("rejects a slug that fails the shape check", () => {
      const source = [
        "```nyxid:connect",
        JSON.stringify({ catalog_slug: "../../etc/passwd" }),
        "```",
      ].join("\n");

      expect(hasConnectMarker(source)).toBe(false);
    });

    it("rejects a missing slug", () => {
      const source = [
        "```nyxid:connect",
        JSON.stringify({ reason: "no slug here" }),
        "```",
      ].join("\n");

      expect(hasConnectMarker(source)).toBe(false);
    });
  });

  describe("streaming", () => {
    it("holds back an unterminated marker mid-stream", () => {
      const partial = [
        "I need GitHub:",
        "```nyxid:connect",
        '{"catalog_slug":"api-git',
      ].join("\n");

      const segments = splitConnectMarkers(partial, { allowPartial: true });

      expect(segments.map((segment) => segment.kind)).toEqual([
        "text",
        "pending",
      ]);
      // The half-written JSON must not reach the transcript.
      expect(JSON.stringify(segments)).not.toContain("api-git");
    });

    it("treats a dangling fence in completed text as literal", () => {
      const partial = ["Text", "```nyxid:connect", '{"catalog_slug":"a'].join(
        "\n",
      );

      const segments = splitConnectMarkers(partial);

      expect(segments.every((segment) => segment.kind === "text")).toBe(true);
    });
  });

  it("caps reason length and scope count", () => {
    const source = [
      "```nyxid:connect",
      JSON.stringify({
        catalog_slug: "api-github",
        reason: "x".repeat(500),
        requested_scopes: Array.from({ length: 30 }, (_, i) => `scope-${i}`),
      }),
      "```",
    ].join("\n");

    const segment = splitConnectMarkers(source)[0];
    expect(segment?.kind).toBe("connect");
    if (segment?.kind !== "connect") return;
    expect(segment.marker.reason).toHaveLength(300);
    expect(segment.marker.requestedScopes).toHaveLength(12);
  });
});

describe("connectMarkerToBlock", () => {
  it("produces a needs_connection card that carries no action inputs", () => {
    const block = connectMarkerToBlock(
      { catalogSlug: "api-github", reason: "read PRs", requestedScopes: [] },
      "connect-1",
    );

    expect(block).toMatchObject({
      type: "connect_card",
      block_id: "connect-1",
      catalog_slug: "api-github",
      state: "needs_connection",
      reason_code: "NYXID_SERVICE_NOT_CONNECTED",
    });
    // `service_name`/`auth_kind` are placeholders — ConnectCard re-resolves
    // both from the catalog before offering an affordance.
    expect(block.service_name).toBe("api-github");
    expect(block.auth_kind).toBe("api_key");
  });
});

describe("renderableText", () => {
  // The transport streams `renderableText(accumulated).slice(emitted.length)`,
  // so any shrink between two prefixes corrupts every subsequent delta. This
  // caught a real one: the newline before a marker belongs to the text run
  // until the opening fence is recognised, then gets absorbed.
  const SAMPLES = [
    'Hello.\n```nyxid:connect\n{"catalog_slug":"api-github"}\n```\nBye.',
    '```nyxid:connect\n{"catalog_slug":"api-github"}\n```',
    'a\r\n```nyxid:connect\r\n{"catalog_slug":"api-github"}\r\n```\r\nb',
    'x\n```nyxid:connect\n{"catalog_slug":"a"}\n```\ny\n```nyxid:connect\n{"catalog_slug":"b"}\n```\nz',
    "text ```inline``` more",
    "```ts\nconst a=1;\n```\nafter",
    'emoji 😀 then\n```nyxid:connect\n{"catalog_slug":"api-github"}\n```',
    "trailing `backtick",
    "no marker at all",
  ];

  it.each(SAMPLES.map((sample, index) => [index, sample] as const))(
    "grows monotonically for sample %i under every chunk boundary",
    (_index, full) => {
      let previous = "";
      for (let cut = 0; cut <= full.length; cut += 1) {
        const current = renderableText(full.slice(0, cut));
        expect(
          current.startsWith(previous),
          `shrank at ${String(cut)}: ${JSON.stringify(previous)} -> ${JSON.stringify(current)}`,
        ).toBe(true);
        previous = current;
      }
    },
  );

  it("never exposes marker syntax", () => {
    for (const sample of SAMPLES) {
      for (let cut = 0; cut <= sample.length; cut += 1) {
        expect(renderableText(sample.slice(0, cut))).not.toContain(
          "nyxid:connect",
        );
      }
    }
  });

  it("keeps prose either side of a marker", () => {
    const full =
      'Before.\n```nyxid:connect\n{"catalog_slug":"api-github"}\n```\nAfter.';
    expect(renderableText(full)).toBe("Before.\nAfter.");
  });
});
