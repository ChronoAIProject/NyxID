import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { crmQueryKeys } from "@/hooks/use-crm";
import type { CrmObject, CrmTableQuery } from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { CrmObjectTable } from "./object-table";
import { CrmPipelineBoard } from "./pipeline-board";

const { mockAssistantJson } = vi.hoisted(() => ({
  mockAssistantJson: vi.fn(),
}));

vi.mock("@/lib/assistant/assistant-http", () => ({
  assistantJson: mockAssistantJson,
}));

const nameField = {
  key: "name",
  label: "Name",
  required: true,
  archived: false,
  type: "text" as const,
  max_length: 1_000,
};

const company: CrmObject = {
  id: "company-object",
  owner_id: "person-1",
  key: "company",
  label: "Company",
  plural_label: "Companies",
  schema_revision: 1,
  fields: [nameField],
  pipeline: null,
  default_columns: ["name"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:00:00Z",
};

const opportunity: CrmObject = {
  ...company,
  id: "opportunity-object",
  key: "opportunity",
  label: "Opportunity",
  plural_label: "Opportunities",
  fields: [
    nameField,
    {
      key: "stage",
      label: "Stage",
      required: true,
      archived: false,
      type: "select",
      options: [
        { value: "new", label: "New", archived: false },
        { value: "qualified", label: "Qualified", archived: false },
      ],
    },
  ],
  pipeline: { stage_field: "stage", closed_stages: [] },
  default_columns: ["name", "stage"],
};

const initialAuth = useAuthStore.getState();

function wrapper(cachedObject?: CrmObject) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  if (cachedObject) {
    client.setQueryData(
      [
        ...crmQueryKeys.object("person-1", cachedObject.key),
        "actor",
        "person-1",
      ],
      cachedObject,
    );
  }
  return ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  useAuthStore.setState({ user: { id: "person-1" } as User });
});

afterEach(() => {
  cleanup();
  useAuthStore.setState(initialAuth);
});

describe("CRM client query validation", () => {
  it("renders an empty lane without fetching when stages do not intersect", async () => {
    mockAssistantJson.mockImplementation((endpoint: string) => {
      if (endpoint === "/assistant/crm/objects/opportunity") {
        return Promise.resolve(opportunity);
      }
      if (endpoint === "/assistant/crm/aggregate") {
        return Promise.resolve({
          points: [],
          unit: { format: "count" },
          total: 0,
          truncated: false,
        });
      }
      if (endpoint === "/assistant/crm/query") {
        return new Promise(() => undefined);
      }
      return Promise.reject(new Error(`Unexpected CRM request: ${endpoint}`));
    });

    render(
      <CrmPipelineBoard
        query={{
          object: "opportunity",
          filters: [{ field: "stage", op: "eq", value: "new" }],
        }}
      />,
      { wrapper: wrapper() },
    );

    const qualifiedLane = await screen.findByRole("region", {
      name: "Qualified",
    });
    expect(within(qualifiedLane).getByText("No records")).toBeInTheDocument();
    expect(within(qualifiedLane).queryByText("Loading...")).not.toBeInTheDocument();

    await waitFor(() => {
      const queryCalls = mockAssistantJson.mock.calls.filter(
        ([endpoint]) => endpoint === "/assistant/crm/query",
      );
      expect(queryCalls).toHaveLength(1);
      expect(queryCalls[0]?.[1]).toEqual({
        method: "POST",
        body: {
          object: "opportunity",
          filters: [{ field: "stage", op: "eq", value: "new" }],
          page_size: 50,
          cursor: undefined,
          include_archived: false,
        },
      });
    });
  });

  it("shows lane errors without fetching an over-limit refined query", async () => {
    mockAssistantJson.mockImplementation((endpoint: string) => {
      if (endpoint === "/assistant/crm/objects/opportunity") {
        return Promise.resolve(opportunity);
      }
      if (endpoint === "/assistant/crm/aggregate") {
        return Promise.resolve({
          points: [],
          unit: { format: "count" },
          total: 0,
          truncated: false,
        });
      }
      return Promise.reject(new Error(`Unexpected CRM request: ${endpoint}`));
    });

    render(
      <CrmPipelineBoard
        query={{
          object: "opportunity",
          filters: Array.from({ length: 20 }, (_, index) => ({
            field: "name",
            op: "contains" as const,
            value: `opportunity-${String(index)}`,
          })),
        }}
      />,
      { wrapper: wrapper() },
    );

    expect(await screen.findAllByText("Could not load this lane.")).toHaveLength(
      2,
    );
    expect(screen.queryByText("Loading...")).not.toBeInTheDocument();
    expect(
      mockAssistantJson.mock.calls.some(
        ([endpoint]) => endpoint === "/assistant/crm/query",
      ),
    ).toBe(false);
  });

  it("shows an error instead of loading forever for an over-limit table query", () => {
    mockAssistantJson.mockImplementation((endpoint: string) => {
      if (endpoint === "/assistant/crm/objects/company") {
        return Promise.resolve(company);
      }
      return Promise.reject(new Error(`Unexpected CRM request: ${endpoint}`));
    });
    const query: CrmTableQuery = {
      object: "company",
      filters: Array.from({ length: 21 }, (_, index) => ({
        field: "name",
        op: "contains" as const,
        value: `company-${String(index)}`,
      })),
    };

    render(<CrmObjectTable query={query} />, { wrapper: wrapper(company) });

    expect(screen.getByText("Could not load records.")).toBeInTheDocument();
    expect(screen.queryByText("Loading records...")).not.toBeInTheDocument();
    expect(mockAssistantJson).toHaveBeenCalledWith(
      "/assistant/crm/objects/company",
    );
    expect(
      mockAssistantJson.mock.calls.some(
        ([endpoint]) => endpoint === "/assistant/crm/query",
      ),
    ).toBe(false);
  });
});
