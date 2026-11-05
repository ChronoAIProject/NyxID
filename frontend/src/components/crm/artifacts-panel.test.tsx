import { expect, it, vi } from "vitest";
import { screen, within } from "@testing-library/react";
import { renderCrmRoute } from "@/test/crm-router";
import type { CrmObject } from "@/schemas/crm";
import { CrmArtifactsPanel } from "./artifacts-panel";

vi.mock("./workspace/crm-views-panel", () => ({
  CrmViewsPanel: () => <p>Saved sales dashboard</p>,
}));

it("groups saved views and custom datasets, excludes seeds and opens the dataset with its owner", async () => {
  const objects = [
    { id: "seed", key: "company", plural_label: "Companies", seeded: true },
    { id: "custom", key: "asset", plural_label: "Assets", seeded: false },
  ] as CrmObject[];
  await renderCrmRoute(
    <CrmArtifactsPanel
      ownerId="person"
      objects={{ data: objects, isPending: false, isError: false }}
    />,
  );
  const datasets = within(
    screen.getByRole("region", { name: "Custom datasets" }),
  );
  expect(datasets.getByRole("link", { name: "Assets" })).toHaveAttribute(
    "href",
    "/assistant/crm?owner=person&object=asset",
  );
  expect(datasets.queryByText("Companies")).not.toBeInTheDocument();
  expect(
    within(screen.getByRole("region", { name: "Saved views" })).getByText(
      "Saved sales dashboard",
    ),
  ).toBeInTheDocument();
});
