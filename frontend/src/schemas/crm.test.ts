import { describe, expect, it } from "vitest";
import {
  crmAggregateQuerySchema,
  crmFieldSchema,
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
});
