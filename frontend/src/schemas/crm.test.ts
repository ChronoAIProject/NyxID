import { describe, expect, it } from "vitest";
import {
  crmActivitiesResponseSchema,
  crmAggregateQuerySchema,
  crmFieldSchema,
  crmSelectOptionSchema,
  crmAggregateResultSchema,
  crmSavedViewEnvelopeSchema,
  crmTableQuerySchema,
} from "./crm";

describe("CRM schemas", () => {
  it("parses flattened object field metadata", () => {
    expect(
      crmFieldSchema.parse({
        key: "amount",
        label: "Amount",
        required: false,
        archived: false,
        type: "currency",
        currency: "USD",
      }),
    ).toMatchObject({ type: "currency", currency: "USD" });
  });

  it("applies bounded table-query defaults", () => {
    expect(
      crmTableQuerySchema.parse({ object: "opportunity", filters: [] }),
    ).toEqual({
      object: "opportunity",
      filters: [],
      page_size: 50,
      include_archived: false,
    });
    expect(() =>
      crmTableQuerySchema.parse({
        object: "opportunity",
        filters: [],
        page_size: 201,
      }),
    ).toThrow();
  });

  it("rejects a malformed aggregate measure", () => {
    expect(
      crmAggregateQuerySchema.safeParse({
        object: "opportunity",
        filters: [],
        measure: { op: "sum" },
        order: "value_desc",
      }).success,
    ).toBe(false);
  });

  it("parses the closed saved-view component catalog", () => {
    const envelope = {
      formatVersion: 1,
      renderer: "json-render",
      rendererSpecVersion: "nyxid-crm-v1",
      queries: {
        openDeals: {
          kind: "table",
          query: { object: "opportunity", filters: [] },
        },
      },
      spec: {
        root: "deals",
        elements: {
          deals: {
            type: "ObjectTable",
            props: { queryId: "openDeals", columns: ["name", "stage"] },
            children: [],
          },
        },
      },
    };
    expect(crmSavedViewEnvelopeSchema.parse(envelope).spec.root).toBe("deals");
    expect(
      crmSavedViewEnvelopeSchema.safeParse({
        ...envelope,
        spec: {
          root: "unsafe",
          elements: {
            unsafe: {
              type: "Iframe",
              props: { url: "https://example.com" },
              children: [],
            },
          },
        },
      }).success,
    ).toBe(false);
  });

  it("accepts the closed RecordForm view component", () => {
    const parsed = crmSavedViewEnvelopeSchema.parse({
      formatVersion: 1,
      renderer: "json-render",
      rendererSpecVersion: "nyxid-crm-v1",
      queries: {},
      spec: {
        root: "createDeal",
        elements: {
          createDeal: {
            type: "RecordForm",
            props: {
              object: "opportunity",
              fields: ["name", "stage"],
              title: "Add an opportunity",
              submitLabel: "Add deal",
            },
            children: [],
          },
        },
      },
    });
    expect(parsed.spec.elements.createDeal?.type).toBe("RecordForm");
  });

  it("parses archived aggregate groups and truncation metadata", () => {
    const result = crmAggregateResultSchema.parse({
      points: [
        { key: "won", label: "Won", value: 4, archived: false },
        { key: "old", label: "Old stage", value: 2, archived: true },
        { key: "__other__", label: "Other", value: 1, archived: false },
      ],
      unit: { format: "count" },
      total: 7,
      truncated: true,
    });
    expect(result.points[1]).toEqual({
      key: "old",
      label: "Old stage",
      value: 2,
      archived: true,
    });
    expect(result.truncated).toBe(true);
  });

  it("preserves boolean groups and absent numeric measurements", () => {
    const result = crmAggregateResultSchema.parse({
      points: [
        { key: true, label: "Yes", value: null, archived: false },
        { key: false, label: "No", value: 12, archived: false },
      ],
      unit: { format: "number", currency: null },
      total: null,
      truncated: false,
    });

    expect(result.points).toEqual([
      { key: true, label: "Yes", value: null, archived: false },
      { key: false, label: "No", value: 12, archived: false },
    ]);
    expect(result.unit.currency).toBeUndefined();
    expect(result.total).toBeNull();
  });

  it("normalizes null activity optionals", () => {
    const result = crmActivitiesResponseSchema.parse({
      activities: [
        {
          id: "activity-1",
          record_id: "record-1",
          object_key: "company",
          kind: "created",
          actor: { kind: "human", user_id: "user-1" },
          summary: { changed_fields: null, from: null, to: null },
          body: null,
          created_at: "2026-11-04T00:00:00Z",
        },
      ],
      next_cursor: null,
    });

    expect(result.activities[0]?.summary).toEqual({
      changed_fields: undefined,
      from: undefined,
      to: undefined,
    });
    expect(result.activities[0]?.body).toBeUndefined();
  });
});

it("accepts the server's empty option color while bounding metadata", () => {
  expect(
    crmSelectOptionSchema.parse({
      value: "_1",
      label: "Option",
      color: "",
      archived: false,
    }).color,
  ).toBe("");
  expect(
    crmSelectOptionSchema.safeParse({
      value: "1_option",
      label: "Option",
      archived: false,
      color: "x".repeat(65),
    }).success,
  ).toBe(false);
});
