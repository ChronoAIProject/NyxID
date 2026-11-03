import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CrmObject } from "@/schemas/crm";
import { useAuthStore } from "@/stores/auth-store";
import { CrmPage } from "./crm-page";

const mocks = vi.hoisted(() => ({
  enabled: false,
  objectTable: vi.fn(),
  useCrmObjects: vi.fn(),
}));

vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => mocks.enabled,
}));
vi.mock("@/hooks/use-crm", () => ({
  useCrmObjects: () => mocks.useCrmObjects(),
}));
vi.mock("@/components/crm/object-table", () => ({
  CrmObjectTable: (props: unknown) => {
    mocks.objectTable(props);
    return <div>CRM table</div>;
  },
}));
vi.mock("@/components/crm/pipeline-board", () => ({
  CrmPipelineBoard: () => null,
}));
vi.mock("@/components/crm/record-sheet", () => ({
  CrmCreateRecordSheet: () => null,
  CrmRecordSheet: () => null,
}));
vi.mock("@/components/crm/workspace/crm-views-panel", () => ({
  CrmViewsPanel: () => null,
}));

const object: CrmObject = {
  id: "company-object",
  owner_id: "user-1",
  key: "company",
  label: "Company",
  plural_label: "Companies",
  schema_revision: 2,
  fields: [
    {
      key: "name",
      label: "Name",
      required: true,
      archived: false,
      type: "text",
      max_length: 1000,
    },
    {
      key: "domain",
      label: "Domain",
      required: false,
      archived: true,
      type: "text",
      max_length: 1000,
    },
  ],
  pipeline: null,
  default_columns: ["name", "domain"],
  seeded: true,
  access: "work",
  created_at: "2026-11-04T00:00:00Z",
  updated_at: "2026-11-04T00:01:00Z",
};

const initialAuth = useAuthStore.getState();

beforeEach(() => {
  mocks.enabled = false;
  mocks.objectTable.mockReset();
  mocks.useCrmObjects.mockReset();
  mocks.useCrmObjects.mockReturnValue({
    data: [object],
    isPending: false,
    isError: false,
    refetch: vi.fn(),
  });
  useAuthStore.setState({ isLoading: false });
  vi.stubGlobal("fetch", vi.fn());
});

afterEach(() => {
  cleanup();
  useAuthStore.setState(initialAuth);
  vi.unstubAllGlobals();
});

it("shows the unavailable state without issuing CRM requests", () => {
  render(<CrmPage />);

  expect(
    screen.getByText("CRM isn't available for your account."),
  ).toBeInTheDocument();
  expect(fetch).not.toHaveBeenCalled();
  expect(mocks.useCrmObjects).not.toHaveBeenCalled();
});

it("removes archived fields from the default query projection", () => {
  mocks.enabled = true;
  render(<CrmPage />);

  expect(screen.getByText("CRM table")).toBeInTheDocument();
  expect(mocks.objectTable).toHaveBeenCalledWith(
    expect.objectContaining({
      query: expect.objectContaining({ select: ["name"] }),
    }),
  );
});
