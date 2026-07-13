import { describe, expect, it } from "vitest";
import {
  adminOAuthClientCustomFilterPatch,
  adminOAuthClientFilterPatch,
  encodeAdminOAuthClientCustomFilters,
  encodeAdminOAuthClientSearchFilters,
  getAdminOAuthClientCustomValues,
  getAppliedAdminOAuthClientFilters,
  getAdminOAuthClientFilterFields,
  getAdminOAuthClientSearchFields,
  getAdminOAuthClientSearchFilters,
  normalizeAdminOAuthClientSearch,
  parseAdminOAuthClientSearchFilters,
  updateAdminOAuthClientSearchFilters,
} from "./admin-oauth-clients";

describe("normalizeAdminOAuthClientSearch", () => {
  it("keeps valid non-default table state", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        page: "3",
        per_page: "50",
        search: "  aevatar  ",
        client_type: "public",
        creator_type: "dynamic_registration",
        broker: "scope",
        is_active: "true",
        scope: "urn:nyxid:scope:broker_binding",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
        sort: "client_name",
      }),
    ).toEqual({
      page: 3,
      per_page: 50,
      search: "aevatar",
      client_type: "public",
      creator_type: "dynamic_registration",
      broker: "scope",
      is_active: true,
      scope: "urn:nyxid:scope:broker_binding",
      created_from: "2026-07-01",
      created_to: "2026-07-31",
      sort: "client_name",
    });
  });

  it("drops defaults and invalid values", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        page: "0",
        per_page: "25",
        search: " ",
        client_type: "native",
        creator_type: "robot",
        broker: "maybe",
        is_active: "yes",
        scope: "profile write",
        created_from: "2026-02-30",
        created_to: "07/31/2026",
        sort: "client_secret_hash",
      }),
    ).toEqual({});
  });

  it("keeps scalar status values boolean-compatible and caps search length", () => {
    expect(normalizeAdminOAuthClientSearch({ is_active: false })).toEqual({
      is_active: false,
    });
    expect(normalizeAdminOAuthClientSearch({ is_active: true })).toEqual({
      is_active: true,
    });
    expect(
      normalizeAdminOAuthClientSearch({ search: "x".repeat(257) }),
    ).toEqual({});
  });

  it("canonicalizes and deduplicates comma-separated structured filters", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        client_type: " confidential,public,confidential ",
        creator_type: "ownerless,system,ownerless",
        broker: "scope,enabled,scope",
        is_active: "false,true,false",
        scope: "profile,openid,profile",
      }),
    ).toEqual({
      client_type: "public,confidential",
      creator_type: "system,ownerless",
      broker: "enabled,scope",
      is_active: "true,false",
      scope: "openid,profile",
    });
  });

  it("orders known scopes first and future scopes lexically", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        scope:
          "urn:nyxid:scope:z_future,profile,urn:nyxid:scope:a_future,openid,urn:nyxid:scope:z_future",
      }),
    ).toEqual({
      scope: "openid,profile,urn:nyxid:scope:a_future,urn:nyxid:scope:z_future",
    });
  });

  it("drops an entire structured filter when a CSV member is invalid", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        client_type: "public,native",
        creator_type: "system,robot",
        broker: "enabled,maybe",
        is_active: "true,yes",
        scope: "openid,profile write",
      }),
    ).toEqual({});

    expect(
      normalizeAdminOAuthClientSearch({
        client_type: "public,,confidential",
        scope: "openid,",
      }),
    ).toEqual({});
  });

  it("keeps scalar structured-filter URLs backward compatible", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        client_type: "confidential",
        creator_type: "dynamic_registration",
        broker: "flag",
        is_active: "false",
        scope: "urn:nyxid:scope:future_capability",
      }),
    ).toEqual({
      client_type: "confidential",
      creator_type: "dynamic_registration",
      broker: "flag",
      is_active: false,
      scope: "urn:nyxid:scope:future_capability",
    });
  });

  it("normalizes strict calendar dates and preserves one-sided ranges", () => {
    expect(
      normalizeAdminOAuthClientSearch({ created_from: "2026-07-03" }),
    ).toEqual({ created_from: "2026-07-03" });
    expect(
      normalizeAdminOAuthClientSearch({ created_to: "2026-07-31" }),
    ).toEqual({ created_to: "2026-07-31" });
    expect(
      normalizeAdminOAuthClientSearch({
        created_from: "2026-02-30",
        created_to: "2026-07-31",
      }),
    ).toEqual({ created_to: "2026-07-31" });
  });

  it("canonicalizes exact dates and gives them precedence over a range", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        created_dates: "2026-07-08, 2026-07-03,2026-07-08",
      }),
    ).toEqual({ created_dates: "2026-07-03,2026-07-08" });
    expect(
      normalizeAdminOAuthClientSearch({
        created_dates: "2026-07-03,2026-07-08",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
      }),
    ).toEqual({ created_dates: "2026-07-03,2026-07-08" });
    expect(
      normalizeAdminOAuthClientSearch({
        created_dates: "2026-02-30,2026-07-08",
      }),
    ).toEqual({});
  });

  it("drops inverted date ranges", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        created_from: "2026-08-01",
        created_to: "2026-07-31",
      }),
    ).toEqual({});
  });

  it("preserves a valid future scope token for server-side validation", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        scope: "urn:nyxid:scope:future_capability",
      }),
    ).toEqual({ scope: "urn:nyxid:scope:future_capability" });
  });

  it("accepts all server-backed column sort values", () => {
    expect(normalizeAdminOAuthClientSearch({ sort: "-broker" })).toEqual({
      sort: "-broker",
    });
    expect(normalizeAdminOAuthClientSearch({ sort: "allowed_scopes" })).toEqual(
      { sort: "allowed_scopes" },
    );
  });

  it("prefers typed server fields and fills missing fields from legacy metadata", () => {
    const fields = getAdminOAuthClientFilterFields({
      client_types: ["public"],
      creator_types: ["system"],
      broker_filters: ["enabled"],
      statuses: [true, false],
      allowed_scopes: ["openid"],
      sorts: ["-created_at"],
      fields: [
        {
          key: "is_active",
          label: "Lifecycle",
          value_type: "boolean",
          operator: "is",
          multiple: true,
          options: [
            { value: "true", label: "On" },
            { value: "false", label: "Off" },
          ],
        },
      ],
    });

    expect(fields).toHaveLength(5);
    expect(fields[0]).toMatchObject({
      key: "is_active",
      label: "Lifecycle",
      value_type: "boolean",
      operator: "is",
      multiple: true,
    });
    expect(fields.find((field) => field.key === "scope")).toMatchObject({
      operator: "includes",
      multiple: false,
      options: [{ value: "openid", label: "OpenID" }],
    });
  });

  it("accepts an advertised date field without adding it to legacy fallbacks", () => {
    expect(getAdminOAuthClientFilterFields()).toHaveLength(5);

    const fields = getAdminOAuthClientFilterFields({
      client_types: ["public"],
      creator_types: ["system"],
      broker_filters: ["enabled"],
      statuses: [true],
      allowed_scopes: ["openid"],
      sorts: ["-created_at"],
      fields: [
        {
          key: "created_at",
          label: "Created",
          value_type: "date",
          operator: "between",
          multiple: true,
          options: [],
        },
      ],
    });

    expect(fields).toHaveLength(6);
    expect(fields[0]).toEqual({
      key: "created_at",
      label: "Created",
      value_type: "date",
      operator: "between",
      multiple: true,
      options: [],
      date_modes: ["dates", "range"],
      max_values: 32,
    });
  });

  it("maps validated value sets into canonical URL state", () => {
    expect(adminOAuthClientFilterPatch("is_active", ["false", "true"])).toEqual(
      {
        is_active: "true,false",
      },
    );
    expect(adminOAuthClientFilterPatch("is_active", ["false"])).toEqual({
      is_active: false,
    });
    expect(
      adminOAuthClientFilterPatch("client_type", [
        "confidential",
        "public",
        "confidential",
      ]),
    ).toEqual({ client_type: "public,confidential" });
    expect(
      adminOAuthClientFilterPatch("client_type", ["public", "native"]),
    ).toBeUndefined();
    expect(
      adminOAuthClientFilterPatch("scope", [
        "urn:nyxid:scope:future_capability",
        "openid",
        "urn:nyxid:scope:future_capability",
      ]),
    ).toEqual({
      scope: "openid,urn:nyxid:scope:future_capability",
    });
    expect(adminOAuthClientFilterPatch("broker", undefined)).toEqual({
      broker: undefined,
    });
    expect(adminOAuthClientFilterPatch("scope", [])).toEqual({
      scope: undefined,
    });
    expect(
      adminOAuthClientFilterPatch("scope", ["openid", "profile write"]),
    ).toBeUndefined();
    expect(
      adminOAuthClientFilterPatch("created_at", [
        "range",
        "2026-07-01",
        "2026-07-31",
      ]),
    ).toEqual({
      created_dates: undefined,
      created_from: "2026-07-01",
      created_to: "2026-07-31",
    });
    expect(
      adminOAuthClientFilterPatch("created_at", [
        "dates",
        "2026-07-08",
        "2026-07-03",
        "2026-07-08",
      ]),
    ).toEqual({
      created_dates: "2026-07-03,2026-07-08",
      created_from: undefined,
      created_to: undefined,
    });
    expect(
      adminOAuthClientFilterPatch("created_at", ["range", "", "2026-07-31"]),
    ).toEqual({
      created_dates: undefined,
      created_from: undefined,
      created_to: "2026-07-31",
    });
    expect(
      adminOAuthClientFilterPatch("created_at", [
        "range",
        "2026-08-01",
        "2026-07-31",
      ]),
    ).toBeUndefined();
    expect(adminOAuthClientFilterPatch("created_at", undefined)).toEqual({
      created_dates: undefined,
      created_from: undefined,
      created_to: undefined,
    });
  });

  it("builds readable applied date filter labels", () => {
    const dateField = {
      key: "created_at",
      label: "Created",
      value_type: "date",
      operator: "between",
      multiple: false,
      options: [],
    } as const;

    expect(
      getAppliedAdminOAuthClientFilters([dateField], {
        created_dates: "2026-07-03,2026-07-08,2026-07-21",
      })[0],
    ).toMatchObject({
      operatorLabel: "is on any of",
      valueSummary: "Jul 3, 2026, Jul 8, 2026 +1 more",
    });
    expect(
      getAppliedAdminOAuthClientFilters([dateField], {
        created_from: "2026-07-01",
        created_to: "2026-07-31",
      })[0],
    ).toMatchObject({
      operatorLabel: "is between",
      valueSummary: "Jul 1, 2026 and Jul 31, 2026",
    });
    expect(
      getAppliedAdminOAuthClientFilters([dateField], {
        created_from: "2026-07-01",
      })[0],
    ).toMatchObject({
      operatorLabel: "is on or after",
      valueSummary: "Jul 1, 2026",
    });
    expect(
      getAppliedAdminOAuthClientFilters([dateField], {
        created_to: "2026-07-31",
      })[0],
    ).toMatchObject({
      operatorLabel: "is on or before",
      valueSummary: "Jul 31, 2026",
    });
  });

  it("retains legacy scalar normalization before multi-select edits", () => {
    expect(
      normalizeAdminOAuthClientSearch({
        client_type: "public",
        is_active: false,
      }),
    ).toEqual({ client_type: "public", is_active: false });
  });

  it("keeps legacy global search normalization for old URLs", () => {
    expect(
      normalizeAdminOAuthClientSearch({ search: "  client, 東京  " }),
    ).toEqual({ search: "client, 東京" });
  });

  it("canonicalizes field searches while preserving commas and Unicode", () => {
    const encoded = encodeAdminOAuthClientSearchFilters([
      { field: "allowed_scopes", values: ["profile,email", "閲覧"] },
      { field: "client", values: ["  Alpha, Beta  ", "Δelta"] },
    ]);

    expect(encoded).toBe(
      '[{"field":"client","values":["Alpha, Beta","Δelta"]},{"field":"allowed_scopes","values":["profile,email","閲覧"]}]',
    );
    expect(parseAdminOAuthClientSearchFilters(encoded)).toEqual([
      { field: "client", values: ["Alpha, Beta", "Δelta"] },
      { field: "allowed_scopes", values: ["profile,email", "閲覧"] },
    ]);
  });

  it("deduplicates same-field OR values case-insensitively", () => {
    const encoded = encodeAdminOAuthClientSearchFilters([
      {
        field: "client",
        values: ["Aevatar", " aevatar ", "Console"],
      },
    ]);

    expect(parseAdminOAuthClientSearchFilters(encoded)).toEqual([
      { field: "client", values: ["Aevatar", "Console"] },
    ]);
  });

  it("orders field groups canonically without changing value order", () => {
    expect(
      parseAdminOAuthClientSearchFilters(
        JSON.stringify([
          { field: "created_by", values: ["second", "first"] },
          { field: "client", values: ["id-2", "id-1"] },
          { field: "client_type", values: ["public"] },
        ]),
      ),
    ).toEqual([
      { field: "client", values: ["id-2", "id-1"] },
      { field: "client_type", values: ["public"] },
      { field: "created_by", values: ["second", "first"] },
    ]);
  });

  it("drops the entire field-search URL state when any group is invalid", () => {
    const invalidValues: unknown[] = [
      "not-json",
      "[]",
      JSON.stringify([{ field: "unknown", values: ["term"] }]),
      JSON.stringify([
        { field: "client", values: ["one"] },
        { field: "client", values: ["two"] },
      ]),
      JSON.stringify([{ field: "client", values: [] }]),
      JSON.stringify([{ field: "client", values: [" "] }]),
      JSON.stringify([{ field: "client", values: ["x".repeat(257)] }]),
      JSON.stringify([
        {
          field: "client",
          values: Array.from({ length: 9 }, (_, index) => String(index)),
        },
      ]),
      JSON.stringify([{ field: "client", values: ["term"], unexpected: true }]),
    ];

    for (const value of invalidValues) {
      expect(parseAdminOAuthClientSearchFilters(value)).toBeUndefined();
      expect(
        normalizeAdminOAuthClientSearch({ search_filters: value }),
      ).toEqual({});
    }
  });

  it("canonicalizes valid field searches during route normalization", () => {
    const search_filters = JSON.stringify([
      { field: "allowed_scopes", values: [" email ", "EMAIL", "openid"] },
      { field: "client", values: [" client,1 "] },
    ]);

    expect(normalizeAdminOAuthClientSearch({ search_filters })).toEqual({
      search_filters:
        '[{"field":"client","values":["client,1"]},{"field":"allowed_scopes","values":["email","openid"]}]',
    });
    expect(
      normalizeAdminOAuthClientSearch({
        search_filters: [
          { field: "created_by", values: [" system "] },
          { field: "client", values: ["portal"] },
        ],
      }),
    ).toEqual({
      search_filters:
        '[{"field":"client","values":["portal"]},{"field":"created_by","values":["system"]}]',
    });
  });

  it("upserts and removes one field-search group", () => {
    const initial = encodeAdminOAuthClientSearchFilters([
      { field: "client", values: ["Aevatar"] },
      { field: "created_by", values: ["system"] },
    ]);
    const updated = updateAdminOAuthClientSearchFilters(initial, "client", [
      "Console",
      "console",
      "Portal",
    ]);

    expect(parseAdminOAuthClientSearchFilters(updated)).toEqual([
      { field: "client", values: ["Console", "Portal"] },
      { field: "created_by", values: ["system"] },
    ]);
    expect(
      updateAdminOAuthClientSearchFilters(updated, "client", undefined),
    ).toBe('[{"field":"created_by","values":["system"]}]');
    expect(
      updateAdminOAuthClientSearchFilters(
        '[{"field":"created_by","values":["system"]}]',
        "created_by",
        [],
      ),
    ).toBeUndefined();
  });

  it("reads canonical field searches from table search state", () => {
    expect(
      getAdminOAuthClientSearchFilters({
        search_filters:
          '[{"field":"client_type","values":["public","confidential"]}]',
      }),
    ).toEqual([{ field: "client_type", values: ["public", "confidential"] }]);
    expect(
      getAdminOAuthClientSearchFilters({ search_filters: "invalid" }),
    ).toEqual([]);
  });

  it("uses advertised search metadata and hides it for legacy backends", () => {
    expect(getAdminOAuthClientSearchFields()).toEqual([]);

    expect(
      getAdminOAuthClientSearchFields({
        client_types: [],
        creator_types: [],
        broker_filters: [],
        statuses: [],
        allowed_scopes: [],
        sorts: [],
        search_fields: [
          { key: "created_by", label: "Provisioned by" },
          { key: "client", label: "Application" },
        ],
      }),
    ).toEqual([
      { key: "client", label: "Application" },
      { key: "created_by", label: "Provisioned by" },
    ]);
  });
});

describe("custom text filters", () => {
  it("canonicalizes custom text into stable, deduplicated URL state", () => {
    // Keys are emitted in a fixed order and values trimmed + deduped
    // case-insensitively, so the same selection always encodes identically.
    expect(
      encodeAdminOAuthClientCustomFilters({
        scope: ["  urn:acme:read  ", "URN:ACME:READ"],
        client_type: ["machine"],
      }),
    ).toBe('{"client_type":["machine"],"scope":["urn:acme:read"]}');

    expect(
      normalizeAdminOAuthClientSearch({
        custom_filters: '{"creator_type":["d0d7b72a"]}',
      }),
    ).toEqual({ custom_filters: '{"creator_type":["d0d7b72a"]}' });
  });

  it("drops custom text for filters with no column to search", () => {
    // is_active is boolean, broker is derived, created_at is a date: the server
    // rejects all three, so they must never reach the URL.
    for (const key of ["is_active", "broker", "created_at", "bogus"]) {
      expect(
        normalizeAdminOAuthClientSearch({
          custom_filters: JSON.stringify({ [key]: ["x"] }),
        }),
      ).toEqual({});
    }
  });

  it("drops malformed, empty, oversized, and over-long custom text", () => {
    for (const raw of [
      "not json",
      "[]",
      "{}",
      '{"scope":[]}',
      '{"scope":["  "]}',
      '{"scope":[7]}',
      `{"scope":["${"a".repeat(257)}"]}`,
      '{"scope":["a","b","c","d","e","f","g","h","i"]}',
    ]) {
      expect(normalizeAdminOAuthClientSearch({ custom_filters: raw })).toEqual(
        {},
      );
    }
  });

  it("reads a filter's custom values back out of URL state", () => {
    const search = normalizeAdminOAuthClientSearch({
      custom_filters: '{"scope":["urn:acme:read"]}',
    });
    expect(getAdminOAuthClientCustomValues(search, "scope")).toEqual([
      "urn:acme:read",
    ]);
    expect(getAdminOAuthClientCustomValues(search, "client_type")).toEqual([]);
  });

  it("clears the param entirely when the last custom value goes away", () => {
    expect(adminOAuthClientCustomFilterPatch({ scope: [] })).toEqual({
      custom_filters: undefined,
    });
  });

  it("gives a filter's custom text its own chip so clearing one keeps the other", () => {
    const fields = getAdminOAuthClientFilterFields({
      client_types: ["public", "confidential"],
      creator_types: ["system"],
      broker_filters: ["enabled"],
      statuses: [true, false],
      allowed_scopes: ["openid"],
      sorts: ["-created_at"],
      fields: [
        {
          key: "client_type",
          label: "Client type",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [{ value: "public", label: "Public" }],
          supports_custom_text: true,
        },
      ],
    });
    expect(fields[0].supports_custom_text).toBe(true);

    const applied = getAppliedAdminOAuthClientFilters(
      fields,
      normalizeAdminOAuthClientSearch({
        client_type: "public",
        custom_filters: '{"client_type":["acme"]}',
      }),
    );
    const clientTypeChips = applied.filter(
      (chip) => chip.field.key === "client_type",
    );
    expect(clientTypeChips).toHaveLength(2);
    expect(clientTypeChips[0].custom).toBeUndefined();
    expect(clientTypeChips[0].valueLabels).toEqual(["Public"]);
    expect(clientTypeChips[1].custom).toBe(true);
    expect(clientTypeChips[1].operatorLabel).toBe("contains");
    expect(clientTypeChips[1].valueLabels).toEqual(["acme"]);
  });

  it("ignores custom text when the server does not advertise support", () => {
    // Rolling deploy: an older backend sends no flag and would 400 on the param.
    const fields = getAdminOAuthClientFilterFields({
      client_types: ["public"],
      creator_types: ["system"],
      broker_filters: ["enabled"],
      statuses: [true],
      allowed_scopes: ["openid"],
      sorts: ["-created_at"],
      fields: [
        {
          key: "client_type",
          label: "Client type",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [{ value: "public", label: "Public" }],
        },
      ],
    });
    expect(fields[0].supports_custom_text).toBe(false);
  });
});
