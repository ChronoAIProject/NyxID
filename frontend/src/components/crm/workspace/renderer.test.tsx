import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmViewRenderer } from "@/components/crm/workspace/renderer";
import type { CrmViewEnvelope } from "@/lib/crm/views/schema";

const { mockAggregate, mockTable } = vi.hoisted(() => ({
  mockAggregate: vi.fn(),
  mockTable: vi.fn(),
}));

vi.mock("@/hooks/use-crm", () => ({
  useCrmAggregate: mockAggregate,
  useCrmTableQuery: mockTable,
  useCrmObject: vi.fn(),
}));

const exampleEnvelope: CrmViewEnvelope = {
  formatVersion: 1,
  renderer: "json-render",
  rendererSpecVersion: "nyxid-crm-v1",
  queries: {
    openDeals: {
      kind: "table",
      query: {
        object: "opportunity",
        filters: [],
        page_size: 50,
      },
    },
    pipeline: {
      kind: "aggregate",
      query: {
        object: "opportunity",
        filters: [],
        group_by: "stage",
        measure: { op: "sum", field: "amount" },
        order: "option_order",
      },
    },
  },
  spec: {
    root: "main",
    elements: {
      main: {
        type: "Stack",
        props: { gap: "md" },
        children: ["title", "chart", "deals"],
      },
      title: {
        type: "Text",
        props: { content: "Sales workspace", variant: "title" },
        children: [],
      },
      chart: {
        type: "BarChart",
        props: {
          queryId: "pipeline",
          title: "Open pipeline by stage",
        },
        children: [],
      },
      deals: {
        type: "ObjectTable",
        props: {
          queryId: "openDeals",
          columns: ["name", "company", "amount", "stage"],
        },
        children: [],
      },
    },
  },
};

beforeEach(() => {
  vi.clearAllMocks();
  mockAggregate.mockReturnValue({
    isPending: false,
    isError: false,
    data: {
      points: [{ key: "qualified", label: "Qualified", value: 125_000 }],
      unit: { format: "currency", currency: "USD" },
      total: 125_000,
    },
  });
  mockTable.mockReturnValue({
    isPending: false,
    isError: false,
    data: {
      object: {
        plural_label: "Opportunities",
        default_columns: ["name"],
        fields: [
          {
            key: "name",
            label: "Name",
            type: "text",
            required: true,
            archived: false,
            max_length: 1_000,
          },
          {
            key: "company",
            label: "Company",
            type: "record",
            required: false,
            archived: false,
            target: "company",
            cardinality: "one",
          },
          {
            key: "amount",
            label: "Amount",
            type: "currency",
            required: false,
            archived: false,
            currency: "USD",
          },
          {
            key: "stage",
            label: "Stage",
            type: "select",
            required: true,
            archived: false,
            options: [
              {
                value: "qualified",
                label: "Qualified",
                archived: false,
              },
            ],
          },
        ],
      },
      rows: [
        {
          id: "deal-1",
          revision: 1,
          values: {
            name: "Acme renewal",
            amount: 125_000,
            stage: "qualified",
          },
          links: {
            company: [
              { id: "company-1", object_key: "company", label: "Acme" },
            ],
          },
          overrides: [],
          archived: false,
        },
      ],
      next_cursor: null,
      total_count: 1,
    },
  });
});

describe("CrmViewRenderer", () => {
  it("renders the contract example with query results bound by queryId", () => {
    render(<CrmViewRenderer envelope={exampleEnvelope} />);

    expect(
      screen.getByRole("heading", { name: "Sales workspace" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "Open pipeline by stage" }),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Acme renewal")).toHaveLength(2);
    expect(screen.getAllByText("Acme")).toHaveLength(2);
    expect(mockAggregate).toHaveBeenCalledWith(
      exampleEnvelope.queries.pipeline!.query,
      undefined,
      true,
    );
  });

  it("renders stored path errors instead of a blank invalid view", () => {
    render(
      <CrmViewRenderer
        envelope={exampleEnvelope}
        valid={false}
        errors={[
          {
            path: "spec.elements.chart.props.queryId",
            message: "Unknown query 'retiredPipeline'",
          },
        ]}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "spec.elements.chart.props.queryId",
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Unknown query 'retiredPipeline'",
    );
    expect(screen.queryByText("Sales workspace")).not.toBeInTheDocument();
  });
});
