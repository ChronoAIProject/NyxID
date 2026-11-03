import { describe, expect, it } from "vitest";
import {
  crmAggregateQuerySchema,
  crmFieldSchema,
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
});
