import { describe, expect, it } from "vitest";
import {
  adminAuditLogCustomFilterPatch,
  adminAuditLogFilterPatch,
  encodeAdminAuditLogSearchFilters,
  getAdminAuditLogCustomValues,
  getAdminAuditLogFilterFields,
  getAdminAuditLogFilterValues,
  getAdminAuditLogSearchFields,
  getAdminAuditLogSearchFilters,
  getAppliedAdminAuditLogFilters,
  isAdminAuditLogDateFilterValid,
  normalizeAdminAuditLogSearch,
  parseAdminAuditLogSearchFilters,
  updateAdminAuditLogSearchFilters,
} from "./admin-audit-log";
import type { AdminAuditLogFilterOptions } from "@/types/admin";

const SERVER_OPTIONS: AdminAuditLogFilterOptions = {
  sorts: ["-created_at", "created_at", "event_type", "-event_type"],
  search_fields: [
    { key: "event_type", label: "Event type" },
    { key: "user_id", label: "User ID" },
    { key: "api_key", label: "Agent / API key" },
    { key: "ip_address", label: "IP address" },
    { key: "user_agent", label: "User agent" },
  ],
  fields: [
    {
      key: "event_type",
      label: "Event type",
      value_type: "enum",
      operator: "is",
      multiple: true,
      supports_custom_text: true,
      options: [
        { value: "login", label: "login" },
        { value: "admin.user.created", label: "admin.user.created" },
      ],
    },
    {
      key: "created_at",
      label: "Created",
      value_type: "date",
      operator: "between",
      multiple: true,
      supports_custom_text: false,
      options: [],
    },
  ],
};

describe("normalizeAdminAuditLogSearch", () => {
  it("keeps valid non-default table state", () => {
    expect(
      normalizeAdminAuditLogSearch({
        page: "3",
        per_page: "50",
        search: "  mfa  ",
        event_type: "login",
        status: "4xx,5xx",
        actor: "agent",
        created_from: "2026-07-01",
        created_to: "2026-07-31",
        sort: "event_type",
      }),
    ).toEqual({
      page: 3,
      per_page: 50,
      search: "mfa",
      event_type: "login",
      status: "4xx,5xx",
      actor: "agent",
      created_from: "2026-07-01",
      created_to: "2026-07-31",
      sort: "event_type",
    });
  });

  it("drops defaults so a clean table has a clean URL", () => {
    expect(
      normalizeAdminAuditLogSearch({
        page: "1",
        per_page: "25",
        sort: "-created_at",
        search: "   ",
      }),
    ).toEqual({});
  });

  it("drops values outside the fixed filter domains", () => {
    expect(
      normalizeAdminAuditLogSearch({
        status: "6xx",
        actor: "robot",
        sort: "password",
        per_page: "7",
      }),
    ).toEqual({});
  });

  it("canonicalizes status order regardless of how it was typed", () => {
    expect(normalizeAdminAuditLogSearch({ status: "5xx,2xx" })).toEqual({
      status: "2xx,5xx",
    });
  });

  it("sorts and dedupes open-vocabulary event types for a stable URL", () => {
    expect(
      normalizeAdminAuditLogSearch({ event_type: "login,admin.x,login" }),
    ).toEqual({ event_type: "admin.x,login" });
  });

  it("drops an inverted created range", () => {
    expect(
      normalizeAdminAuditLogSearch({
        created_from: "2026-07-31",
        created_to: "2026-07-01",
      }),
    ).toEqual({});
  });

  it("prefers created_dates over a range when both are present", () => {
    expect(
      normalizeAdminAuditLogSearch({
        created_dates: "2026-07-02,2026-07-01",
        created_from: "2026-07-01",
      }),
    ).toEqual({ created_dates: "2026-07-01,2026-07-02" });
  });
});

describe("scoped search filters", () => {
  it("round-trips a parsed group", () => {
    const encoded = encodeAdminAuditLogSearchFilters([
      { field: "user_id", values: ["u1"] },
    ]);
    expect(encoded).toBe('[{"field":"user_id","values":["u1"]}]');
    expect(parseAdminAuditLogSearchFilters(encoded)).toEqual([
      { field: "user_id", values: ["u1"] },
    ]);
  });

  it("rejects an unknown field", () => {
    expect(
      parseAdminAuditLogSearchFilters('[{"field":"event_data","values":["x"]}]'),
    ).toBeUndefined();
  });

  it("rejects a group that exceeds the per-field value cap", () => {
    const values = Array.from({ length: 9 }, (_, i) => `v${String(i)}`);
    expect(
      encodeAdminAuditLogSearchFilters([{ field: "user_id", values }]),
    ).toBeUndefined();
  });

  it("removes a field group when its last value is cleared", () => {
    const current = '[{"field":"user_id","values":["u1"]}]';
    expect(
      updateAdminAuditLogSearchFilters(current, "user_id", undefined),
    ).toBeUndefined();
  });

  it("reads applied groups off table state", () => {
    expect(
      getAdminAuditLogSearchFilters({
        search_filters: '[{"field":"api_key","values":["coding"]}]',
      }),
    ).toEqual([{ field: "api_key", values: ["coding"] }]);
  });
});

describe("custom-text filters", () => {
  it("encodes text typed into the event-type filter", () => {
    expect(
      adminAuditLogCustomFilterPatch({ event_type: ["mfa"] }),
    ).toEqual({ custom_filters: '{"event_type":["mfa"]}' });
  });

  it("drops filters that carry no text", () => {
    expect(
      adminAuditLogCustomFilterPatch({ event_type: [], status: ["4xx"] }),
    ).toEqual({ custom_filters: undefined });
  });

  it("ignores custom text on a derived filter the server cannot match", () => {
    expect(
      normalizeAdminAuditLogSearch({
        custom_filters: '{"status":["4xx"]}',
      }),
    ).toEqual({});
  });

  it("reads custom values off table state", () => {
    expect(
      getAdminAuditLogCustomValues(
        { custom_filters: '{"event_type":["mfa"]}' },
        "event_type",
      ),
    ).toEqual(["mfa"]);
  });

  it("encodes text typed into a text-only column filter", () => {
    expect(
      adminAuditLogCustomFilterPatch({
        user_id: ["b9047537"],
        ip_address: ["10.0.0"],
      }),
    ).toEqual({
      custom_filters: '{"user_id":["b9047537"],"ip_address":["10.0.0"]}',
    });
  });

  it("survives text in every text filter at once", () => {
    const patch = adminAuditLogCustomFilterPatch({
      event_type: ["login"],
      api_key_name: ["codex"],
      user_id: ["u"],
      api_key_id: ["k"],
      ip_address: ["10."],
      user_agent: ["Mozilla"],
    });
    expect(patch.custom_filters).toBeDefined();
    expect(
      Object.keys(
        JSON.parse(patch.custom_filters as string) as Record<string, unknown>,
      ),
    ).toHaveLength(6);
  });

  it("keeps a text filter out of the query params it does not own", () => {
    // Its whole state lives in custom_filters.
    expect(adminAuditLogFilterPatch("user_id", ["x"])).toEqual({});
  });
});

describe("adminAuditLogFilterPatch", () => {
  it("clears every created_at param when the date filter is emptied", () => {
    expect(adminAuditLogFilterPatch("created_at", undefined)).toEqual({
      created_dates: undefined,
      created_from: undefined,
      created_to: undefined,
    });
  });

  it("maps the range mode onto created_from / created_to", () => {
    expect(
      adminAuditLogFilterPatch("created_at", [
        "range",
        "2026-07-01",
        "2026-07-31",
      ]),
    ).toEqual({
      created_dates: undefined,
      created_from: "2026-07-01",
      created_to: "2026-07-31",
    });
  });

  it("rejects an inverted range", () => {
    expect(
      isAdminAuditLogDateFilterValid(["range", "2026-07-31", "2026-07-01"]),
    ).toBe(false);
  });

  it("rejects a status outside the fixed domain", () => {
    expect(adminAuditLogFilterPatch("status", ["6xx"])).toBeUndefined();
  });

  it("accepts an event type the server never advertised", () => {
    // The vocabulary is open, so a value can be legitimate without being one of
    // the checkbox options.
    expect(adminAuditLogFilterPatch("event_type", ["brand.new.event"])).toEqual({
      event_type: "brand.new.event",
    });
  });
});

describe("getAdminAuditLogFilterFields", () => {
  it("offers one filter per table column, in column order", () => {
    const fields = getAdminAuditLogFilterFields(SERVER_OPTIONS);
    expect(fields.map((field) => field.key)).toEqual([
      "event_type",
      "status",
      "actor",
      "created_at",
      "api_key_name",
      "user_id",
      "api_key_id",
      "ip_address",
      "user_agent",
    ]);
    expect(fields[0]?.options).toHaveLength(2);
    expect(fields[0]?.supports_custom_text).toBe(true);
  });

  it("makes the unbounded columns text-only filters", () => {
    const fields = getAdminAuditLogFilterFields(SERVER_OPTIONS);
    const userId = fields.find((field) => field.key === "user_id");
    expect(userId?.value_type).toBe("text");
    expect(userId?.operator).toBe("contains");
    expect(userId?.options).toEqual([]);
    expect(userId?.supports_custom_text).toBe(true);
  });

  it("falls back to the statically-known filters without a server payload", () => {
    // `event_type` has no client-side option list, so it stays hidden until the
    // server advertises one; the text filters need no options at all.
    expect(getAdminAuditLogFilterFields().map((field) => field.key)).toEqual([
      "status",
      "actor",
      "api_key_name",
      "user_id",
      "api_key_id",
      "ip_address",
      "user_agent",
    ]);
  });

  it("drops a text filter the server will not match text against", () => {
    const fields = getAdminAuditLogFilterFields({
      fields: [
        {
          key: "user_id",
          label: "User ID",
          value_type: "text",
          operator: "contains",
          multiple: false,
          options: [],
          supports_custom_text: false,
        },
      ],
    });
    // Falls back to the client-side text field rather than rendering a filter
    // with no way to enter a value.
    expect(fields.find((f) => f.key === "user_id")?.supports_custom_text).toBe(
      true,
    );
  });

  it("hides the custom-text box when an older server omits the flag", () => {
    const fields = getAdminAuditLogFilterFields({
      fields: [
        {
          key: "event_type",
          label: "Event type",
          value_type: "enum",
          operator: "is",
          multiple: true,
          options: [{ value: "login", label: "login" }],
        },
      ],
    });
    expect(fields[0]?.supports_custom_text).toBe(false);
  });
});

describe("getAdminAuditLogSearchFields", () => {
  it("returns server fields in table order", () => {
    expect(
      getAdminAuditLogSearchFields(SERVER_OPTIONS).map((field) => field.key),
    ).toEqual(["event_type", "user_id", "api_key", "ip_address", "user_agent"]);
  });

  it("hides scoped search for a server that does not advertise it", () => {
    expect(getAdminAuditLogSearchFields()).toEqual([]);
  });
});

describe("getAppliedAdminAuditLogFilters", () => {
  it("labels checked options and custom text as separate chips", () => {
    const fields = getAdminAuditLogFilterFields(SERVER_OPTIONS);
    const applied = getAppliedAdminAuditLogFilters(fields, {
      event_type: "login",
      custom_filters: '{"event_type":["mfa"]}',
    });

    expect(applied).toHaveLength(2);
    expect(applied[0]?.valueLabels).toEqual(["login"]);
    expect(applied[0]?.custom).toBeUndefined();
    expect(applied[1]?.operatorLabel).toBe("contains");
    expect(applied[1]?.custom).toBe(true);
  });

  it("summarizes a date range", () => {
    const fields = getAdminAuditLogFilterFields(SERVER_OPTIONS);
    const applied = getAppliedAdminAuditLogFilters(fields, {
      created_from: "2026-07-01",
      created_to: "2026-07-31",
    });
    expect(applied[0]?.operatorLabel).toBe("is between");
    expect(applied[0]?.valueSummary).toBe("Jul 1, 2026 and Jul 31, 2026");
  });

  it("reads status selections back as filter values", () => {
    expect(
      getAdminAuditLogFilterValues({ status: "4xx,5xx" }, "status"),
    ).toEqual(["4xx", "5xx"]);
  });
});
