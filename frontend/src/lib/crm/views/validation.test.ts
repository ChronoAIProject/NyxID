import { describe, expect, it } from "vitest";
import { validateCrmViewEnvelope } from "@/lib/crm/views/validation";

function envelopeWith(element: unknown) {
  return {
    formatVersion: 1,
    renderer: "json-render",
    rendererSpecVersion: "nyxid-crm-v1",
    queries: {},
    spec: { root: "main", elements: { main: element } },
  };
}

describe("validateCrmViewEnvelope", () => {
  it("rejects an unknown component at its type path", () => {
    const result = validateCrmViewEnvelope(
      envelopeWith({ type: "Iframe", props: {}, children: [] }),
    );

    expect(result).toEqual({
      success: false,
      errors: [
        {
          path: "spec.elements.main.type",
          message: "Unknown component 'Iframe'",
        },
      ],
    });
  });

  it("rejects an unknown prop at its exact path", () => {
    const result = validateCrmViewEnvelope(
      envelopeWith({
        type: "Text",
        props: { content: "Hello", variant: "body", html: "<b>Hi</b>" },
        children: [],
      }),
    );

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.errors).toContainEqual({
        path: "spec.elements.main.props",
        message: "Unrecognized key: \"html\"",
      });
    }
  });

  it("enforces component query kind and aggregate shape", () => {
    const result = validateCrmViewEnvelope({
      ...envelopeWith({
        type: "Metric",
        props: { queryId: "totals", title: "Total" },
        children: [],
      }),
      queries: {
        totals: {
          kind: "aggregate",
          query: {
            object: "opportunity",
            filters: [],
            group_by: "stage",
            measure: { op: "count" },
            order: "option_order",
          },
        },
      },
    });

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.errors).toContainEqual({
        path: "spec.elements.main.props.queryId",
        message: "Metric requires an aggregate query without group_by",
      });
    }
  });

  it("rejects invalid element and query ids at their key paths", () => {
    const result = validateCrmViewEnvelope({
      formatVersion: 1,
      renderer: "json-render",
      rendererSpecVersion: "nyxid-crm-v1",
      queries: {
        "1bad": {
          kind: "table",
          query: { object: "company", filters: [] },
        },
      },
      spec: {
        root: "1root",
        elements: {
          "1root": { type: "Text", props: { content: "Hi", variant: "body" }, children: [] },
        },
      },
    });

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.errors.map((error) => error.path)).toEqual(
        expect.arrayContaining([
          "queries.1bad",
          "spec.root",
          "spec.elements.1root",
        ]),
      );
    }
  });

  it("rejects archived or missing fields selected by data components", () => {
    const result = validateCrmViewEnvelope(
      {
        formatVersion: 1,
        renderer: "json-render",
        rendererSpecVersion: "nyxid-crm-v1",
        queries: {
          companies: {
            kind: "table",
            query: { object: "company", filters: [] },
          },
        },
        spec: {
          root: "main",
          elements: {
            main: {
              type: "Stack",
              props: { gap: "md" },
              children: ["table", "form"],
            },
            table: {
              type: "ObjectTable",
              props: { queryId: "companies", columns: ["retired"] },
              children: [],
            },
            form: {
              type: "RecordForm",
              props: {
                object: "company",
                fields: ["missing"],
                title: "Add company",
              },
              children: [],
            },
          },
        },
      },
      [
        {
          key: "company",
          fields: [
            { key: "name", archived: false },
            { key: "retired", archived: true },
          ],
        },
      ],
    );

    expect(result.success).toBe(false);
    if (!result.success) {
      expect(result.errors).toEqual(
        expect.arrayContaining([
          {
            path: "spec.elements.table.props.columns.0",
            message: "Must reference a live field on 'company'",
          },
          {
            path: "spec.elements.form.props.fields.0",
            message: "Must reference a live field on 'company'",
          },
        ]),
      );
    }
  });
});
