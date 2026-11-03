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
});
