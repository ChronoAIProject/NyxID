import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CrmViewRenderer } from "@/components/crm/workspace/renderer";
import type { CrmViewEnvelope } from "@/lib/crm/views/schema";

const { mockAggregate, mockTable, mockObject, mockRecordForm } = vi.hoisted(() => ({
  mockAggregate: vi.fn(),
  mockTable: vi.fn(),
  mockObject: vi.fn(),
  mockRecordForm: vi.fn(),
}));

vi.mock("@/hooks/use-crm", () => ({
  crmQueryKeys: { root: (owner: string | undefined) => ["crm", owner] },
  useCrmAggregate: mockAggregate,
  useCrmTableQuery: mockTable,
  useCrmObject: mockObject,
}));

vi.mock("@/components/crm/record-form", () => ({
  CrmRecordForm: (props: { readonly title: string }) => {
    mockRecordForm(props);
    return <div>{props.title}</div>;
  },
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
      points: [
        {
          key: "qualified",
          label: "Qualified",
          value: 125_000,
          archived: false,
        },
      ],
      unit: { format: "currency", currency: "USD" },
      total: 125_000,
      truncated: false,
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
  mockObject.mockReturnValue({
    isPending: false,
    isError: false,
    data: {
      key: "company",
      access: "work",
      fields: [{ key: "name", archived: false, required: true }],
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

  it("notes a truncated aggregate with an Other remainder", () => {
    mockAggregate.mockReturnValue({
      isPending: false,
      isError: false,
      data: {
        points: [
          {
            key: "__other__",
            label: "Other",
            value: 25_000,
            archived: false,
          },
        ],
        unit: { format: "currency", currency: "USD" },
        total: 25_000,
        truncated: true,
      },
    });

    render(<CrmViewRenderer envelope={exampleEnvelope} />);

    expect(
      screen.getByText("Additional groups are combined into Other."),
    ).toBeInTheDocument();
  });

  it("renders RecordForm for work access and invalidates CRM queries on create", async () => {
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const recordFormEnvelope: CrmViewEnvelope = {
      formatVersion: 1,
      renderer: "json-render",
      rendererSpecVersion: "nyxid-crm-v1",
      queries: {},
      spec: {
        root: "form",
        elements: {
          form: {
            type: "RecordForm",
            props: {
              object: "company",
              fields: ["name"],
              title: "Add company",
              submitLabel: "Add",
            },
            children: [],
          },
        },
      },
    };
    render(
      <QueryClientProvider client={queryClient}>
        <CrmViewRenderer envelope={recordFormEnvelope} ownerId="owner-1" />
      </QueryClientProvider>,
    );

    expect(screen.getByText("Add company")).toBeInTheDocument();
    const props = mockRecordForm.mock.calls[0]?.[0] as {
      readonly onCreated: () => void;
    };
    props.onCreated();
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: ["crm", "owner-1"],
    });
  });

  it("hides RecordForm for read access", () => {
    mockObject.mockReturnValue({
      isPending: false,
      isError: false,
      data: {
        key: "company",
        access: "read",
        fields: [{ key: "name", archived: false, required: true }],
      },
    });
    const recordFormEnvelope: CrmViewEnvelope = {
      formatVersion: 1,
      renderer: "json-render",
      rendererSpecVersion: "nyxid-crm-v1",
      queries: {},
      spec: {
        root: "form",
        elements: {
          form: {
            type: "RecordForm",
            props: { object: "company", title: "Add company" },
            children: [],
          },
        },
      },
    };

    render(<CrmViewRenderer envelope={recordFormEnvelope} />);

    expect(screen.queryByText("Add company")).not.toBeInTheDocument();
    expect(mockRecordForm).not.toHaveBeenCalled();
  });
});
