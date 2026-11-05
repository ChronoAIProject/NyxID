import { render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import CrmArtifacts from "./artifacts";
import { CrmArtifactBlock } from "@/components/assistant/blocks/crm-artifact-block";
import { ApiError } from "@/lib/api-client";
import type { TurnCrmArtifact } from "@/schemas/assistant-nyxagent";
const state = vi.hoisted(() => ({
  enabled: true,
  record: {} as Record<string, unknown>,
  object: {} as Record<string, unknown>,
  view: {} as Record<string, unknown>,
  count: {} as Record<string, unknown>,
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => state.enabled,
}));
vi.mock("@/hooks/use-crm", () => ({
  useCrmRecord: () => state.record,
  useCrmObject: () => state.object,
  useCrmAggregate: () => state.count,
}));
vi.mock("@/hooks/use-crm-views", () => ({ useCrmView: () => state.view }));
vi.mock("./workspace/renderer", () => ({
  CrmViewRenderer: () => <p>Live view contents</p>,
}));
const artifact: TurnCrmArtifact = {
  kind: "record",
  id: "r",
  owner_id: "owner",
  object_key: "company",
  title: "Stale message title",
  role: "created",
};
beforeEach(() => {
  state.enabled = true;
  state.record = {};
  state.object = {};
  state.view = {};
  state.count = {};
});
it("renders nothing when the person has the flag off", () => {
  state.enabled = false;
  const view = render(<CrmArtifactBlock artifacts={[artifact]} />);
  expect(view.container).toBeEmptyDOMElement();
});
it("uses the viewer's live record instead of values or titles from the message", () => {
  state.record = {
    data: {
      id: "r",
      object_key: "company",
      values: { name: "Live name" },
      archived: false,
    },
  };
  state.object = {
    data: {
      label: "Company",
      fields: [{ key: "name", label: "Name", required: true, type: "text" }],
    },
  };
  render(<CrmArtifacts artifacts={[artifact]} />);
  expect(
    screen.getByRole("heading", { name: "Live name" }),
  ).toBeInTheDocument();
  expect(screen.queryByText("Stale message title")).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "Open" })).toHaveAttribute(
    "href",
    "/assistant/crm?owner=owner&object=company&record=r",
  );
});
it.each([403, 404, 410])(
  "hides cached record data after %s and shows unavailable",
  (status) => {
    state.record = {
      isError: true,
      data: { values: { name: "Cached secret" } },
      error: new ApiError(status, {
        error: "not_found",
        message: "Unavailable",
        error_code: 1,
      }),
    };
    render(<CrmArtifacts artifacts={[artifact]} />);
    expect(screen.getByRole("alert")).toHaveTextContent(
      "This artifact is no longer available",
    );
    expect(screen.queryByText("Cached secret")).not.toBeInTheDocument();
  },
);
it("shows loading and connection errors explicitly", () => {
  const view = render(<CrmArtifacts artifacts={[artifact]} />);
  expect(screen.getByRole("status")).toHaveTextContent("Loading artifact");
  state.record = { isError: true, error: new Error("offline") };
  view.rerender(<CrmArtifacts artifacts={[artifact]} />);
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Could not load this artifact",
  );
});
it("renders a saved view and an empty dataset with reopen links", () => {
  state.view = {
    data: { title: "Live saved view", envelope: {}, valid: true, errors: [] },
  };
  state.object = { data: { key: "asset", plural_label: "Assets" } };
  state.count = { data: { points: [{ value: 0 }] } };
  render(
    <CrmArtifacts
      artifacts={[
        { ...artifact, kind: "view" },
        { ...artifact, id: "o", kind: "object", object_key: "asset" },
      ]}
    />,
  );
  expect(screen.getByText("Live view contents")).toBeInTheDocument();
  expect(screen.getByText("No records yet.")).toBeInTheDocument();
});
