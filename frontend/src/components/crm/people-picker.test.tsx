import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { CrmObjectTable } from "./object-table";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
const { request } = vi.hoisted(() => ({ request: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: request }));
it("resolves 200 visible assignees in one owner-scoped batch with no per-cell requests", async () => {
  useAuthStore.setState({ user: { id: "person-a" } as User });
  const ids = Array.from({ length: 200 }, (_, index) => `member-${index}`);
  const object = {
    id: "object",
    owner_id: "org-a",
    key: "company",
    label: "Company",
    plural_label: "Companies",
    schema_revision: 1,
    fields: [
      {
        key: "name",
        label: "Name",
        type: "text",
        max_length: 1000,
        required: true,
        archived: false,
      },
      {
        key: "owner",
        label: "Owner",
        type: "user",
        required: false,
        archived: false,
      },
    ],
    pipeline: null,
    default_columns: ["name", "owner"],
    seeded: true,
    access: "work",
    created_at: "2026-11-04T00:00:00Z",
    updated_at: "2026-11-04T00:00:00Z",
  };
  request.mockImplementation(async (path: string) => {
    if (path.includes("people/lookup"))
      return {
        people: ids.map((id, index) => ({
          id,
          label: index === 199 ? "Unavailable member" : `Person ${index}`,
          available: index !== 199,
        })),
      };
    if (path.includes("/objects/")) return object;
    if (path.endsWith("/query"))
      return {
        object,
        rows: ids.map((id, index) => ({
          id: `record-${index}`,
          revision: 1,
          values: { name: `Company ${index}`, owner: id },
          links: {},
          overrides: [],
          archived: false,
        })),
        next_cursor: null,
        total_count: 200,
      };
    throw new Error(`Unexpected request: ${path}`);
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <CrmObjectTable
        query={{ object: "company", filters: [], page_size: 200 }}
        ownerId="org-a"
      />
    </QueryClientProvider>,
  );
  await screen.findAllByText("Person 198");
  await waitFor(() =>
    expect(
      request.mock.calls.filter(([path]) =>
        String(path).includes("people/lookup"),
      ),
    ).toHaveLength(1),
  );
  const lookup = request.mock.calls.find(([path]) =>
    String(path).includes("people/lookup"),
  );
  expect(lookup?.[1].body.owner_id).toBe("org-a");
  expect(lookup?.[1].body.ids).toHaveLength(200);
  expect(
    request.mock.calls.some(([path]) => /people\/member-/.test(String(path))),
  ).toBe(false);
  expect(screen.getAllByText("Unavailable member").length).toBeGreaterThan(0);
});
