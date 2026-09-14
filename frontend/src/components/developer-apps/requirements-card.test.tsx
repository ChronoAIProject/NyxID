import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  publish: vi.fn(),
  data: { versions: [], validator_profiles: [] },
}));
vi.mock("@/hooks/use-app-requirements", () => ({
  useAppRequirementManifests: () => ({ data: mocks.data, isPending: false }),
  usePublishAppRequirements: () => ({
    mutateAsync: mocks.publish,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalog: () => ({ data: [{ slug: "api-github", name: "GitHub" }] }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
import { RequirementsCard } from "./requirements-card";

beforeEach(() => {
  vi.clearAllMocks();
  mocks.publish.mockResolvedValue({ version: 1 });
});

it("publishes a catalog requirement with form policies and no gate option", async () => {
  const user = userEvent.setup();
  render(<RequirementsCard clientId="app" />);
  await user.click(screen.getByRole("button", { name: "Publish new version" }));
  await user.click(screen.getByRole("button", { name: "Add requirement" }));
  await user.type(screen.getByLabelText("ID"), "github");
  await user.type(
    screen.getByLabelText("Label", { exact: true }),
    "Source code",
  );
  await user.click(screen.getByRole("checkbox", { name: /GitHub/ }));
  await user.click(screen.getByRole("checkbox", { name: "oauth2" }));
  await user.type(
    screen.getByLabelText("Required downstream OAuth scopes"),
    "repo read:user",
  );
  await user.click(
    screen.getByRole("button", { name: "Publish version" }),
  );
  await waitFor(() =>
    expect(mocks.publish).toHaveBeenCalledWith({
      enforcement: "advise",
      requirements: [
        {
          id: "github",
          label: "Source code",
          any_of_catalog_slugs: ["api-github"],
          any_of_catalog_prefix: null,
          owner_policy: "personal_only",
          accepted_credential_types: ["oauth2"],
          allow_master_credential: false,
          allow_no_credential: false,
          required_downstream_scopes: ["repo", "read:user"],
          validator: { kind: "stored_only" },
          optional: false,
        },
      ],
    }),
  );
  expect(
    screen.queryByRole("option", { name: "Gate" }),
  ).not.toBeInTheDocument();
});

it("accepts a frozen-prefix request and prevents invalid requirement publication", async () => {
  const user = userEvent.setup();
  render(<RequirementsCard clientId="app" />);
  await user.click(screen.getByRole("button", { name: "Publish new version" }));
  await user.click(screen.getByRole("button", { name: "Add requirement" }));
  await user.click(
    screen.getByRole("button", { name: "Publish version" }),
  );
  expect(mocks.publish).not.toHaveBeenCalled();
  expect(await screen.findByRole("alert")).toHaveTextContent("Cannot save");
  await user.type(screen.getByLabelText("ID"), "llm");
  await user.type(screen.getByLabelText("Label", { exact: true }), "LLM");
  await user.type(screen.getByLabelText("Catalog prefix (optional)"), "llm-");
  await user.click(
    screen.getByRole("button", { name: "Publish version" }),
  );
  await waitFor(() =>
    expect(mocks.publish).toHaveBeenCalledWith(
      expect.objectContaining({
        requirements: [
          expect.objectContaining({
            any_of_catalog_prefix: "llm-",
            any_of_catalog_slugs: [],
          }),
        ],
      }),
    ),
  );
});
