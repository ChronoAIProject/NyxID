import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ApiKeyCreateDialog } from "./api-key-create-dialog";
import { AgentKeyCreateForm } from "../auth/agent-key-create-form";

const { create, services } = vi.hoisted(() => ({
  create: vi.fn(),
  services: [
    {
      id: "personal-service",
      label: "Personal service",
      is_active: true,
      credential_source: { type: "personal" },
    },
  ],
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useCreateApiKey: () => ({ mutateAsync: create, isPending: false }),
}));
vi.mock("@/hooks/use-keys", () => ({ useKeys: () => ({ data: services }) }));
vi.mock("@/hooks/use-nodes", () => ({ useNodes: () => ({ data: [] }) }));
const orgs = [{ id: "org", display_name: "Organization", your_role: "admin" }];
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: () => ({ data: orgs }) }));
vi.mock("@/components/shared/org-scope-select", () => ({
  OrgScopeSelect: ({
    value,
    onChange,
  }: {
    value: string | null;
    onChange: (value: string | null) => void;
  }) => (
    <select
      aria-label="Owner"
      value={value ?? ""}
      onChange={(event) => onChange(event.target.value || null)}
    >
      <option value="">Personal</option>
      <option value="org">Organization</option>
    </select>
  ),
}));
vi.mock("@/components/ui/date-picker", () => ({
  DatePicker: ({
    value,
    onChange,
  }: {
    value: string | null;
    onChange: (value: string | null) => void;
  }) => (
    <input
      aria-label="Expiry date"
      value={value ?? ""}
      onChange={(event) => onChange(event.target.value || null)}
    />
  ),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: React.ReactNode }) => (
    <span>{children}</span>
  ),
}));

beforeEach(() => {
  vi.clearAllMocks();
  create.mockResolvedValue({
    id: "key",
    full_key: "fixture",
    expires_at: "2099-01-01T00:00:00Z",
  });
});

function restrictedFinite() {
  expect(
    screen.getByRole("checkbox", { name: "Allow all services" }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: "Allow all nodes" }),
  ).not.toBeChecked();
  expect(screen.getByRole("checkbox", { name: "No expiry" })).not.toBeChecked();
  const date = (screen.getByLabelText("Expiry date") as HTMLInputElement).value;
  expect(Date.parse(date)).toBeGreaterThan(Date.now() + 89 * 86400000);
}

describe("new dashboard key intent", () => {
  it("opens restricted and finite, resets on close, and resets after both owner transitions", async () => {
    const user = userEvent.setup();
    render(<ApiKeyCreateDialog />);
    await user.click(screen.getByRole("button", { name: "Create API Key" }));
    restrictedFinite();
    await user.click(screen.getByRole("checkbox", { name: "No expiry" }));
    await user.click(
      screen.getByRole("checkbox", { name: "Allow all services" }),
    );
    await user.click(screen.getByRole("checkbox", { name: "Allow all nodes" }));
    await user.selectOptions(screen.getByLabelText("Owner"), "org");
    restrictedFinite();
    await user.click(screen.getByRole("checkbox", { name: "No expiry" }));
    await user.selectOptions(screen.getByLabelText("Owner"), "");
    restrictedFinite();
    await user.click(screen.getByRole("checkbox", { name: "No expiry" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "Create API Key" }));
    restrictedFinite();
    await user.type(screen.getByLabelText("Name"), "finite");
    await user.click(screen.getByRole("checkbox", { name: "read" }));
    await user.click(screen.getByRole("button", { name: "Create Key" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          allow_all_services: false,
          allow_all_nodes: false,
          never_expires: false,
        }),
      ),
    );
    expect(await screen.findByText(/Expires/)).toHaveTextContent("2099");
  });

  it("setup retains only an owner-valid initial service and permits explicit nonexpiry", async () => {
    const user = userEvent.setup();
    render(
      <ApiKeyCreateDialog
        externalOpen
        setupMode
        initialServiceId="personal-service"
      />,
    );
    restrictedFinite();
    expect(
      screen.getByRole("checkbox", { name: "Personal service" }),
    ).toBeChecked();
    await user.click(screen.getByRole("checkbox", { name: "No expiry" }));
    await user.click(screen.getByRole("button", { name: "Create Agent Key" }));
    await waitFor(() =>
      expect(create).toHaveBeenCalledWith(
        expect.objectContaining({
          expires_at: null,
          never_expires: true,
          allow_all_services: false,
          allow_all_nodes: false,
          allowed_service_ids: ["personal-service"],
          allowed_node_ids: [],
        }),
      ),
    );
  });
});

it("login form resets explicit nonexpiry and broad grants on owner changes with a truthful selector", async () => {
  const user = userEvent.setup();
  const review = vi.fn();
  render(
    <AgentKeyCreateForm
      label="Login"
      disabled={false}
      onReview={review}
      initialValues={{
        name: "Login",
        scopes: ["proxy"],
        never_expires: true,
        expires_at: null,
        allow_all_services: true,
        allow_all_nodes: true,
      }}
      options={{
        keys: [],
        services: [],
        nodes: [],
        orgs: [{ id: "org", name: "Organization", owner_id: "org" }],
      }}
    />,
  );
  expect(
    screen.getByRole("combobox", { name: "Key expiry" }),
  ).toHaveTextContent("No expiry");
  await user.click(screen.getAllByRole("combobox")[0]!);
  await user.click(screen.getByRole("option", { name: "Organization" }));
  expect(
    screen.getByRole("combobox", { name: "Key expiry" }),
  ).toHaveTextContent("90 days");
  expect(
    screen.getByRole("checkbox", { name: "Allow all services" }),
  ).not.toBeChecked();
  expect(
    screen.getByRole("checkbox", { name: "Allow all nodes" }),
  ).not.toBeChecked();
  await user.click(screen.getAllByRole("combobox")[0]!);
  await user.click(screen.getByRole("option", { name: "Personal" }));
  expect(
    screen.getByRole("combobox", { name: "Key expiry" }),
  ).toHaveTextContent("90 days");
  await user.click(screen.getByRole("button", { name: /Review/ }));
  await waitFor(() => expect(review).toHaveBeenCalled());
  expect(review.mock.calls[0]![0]).toMatchObject({
    never_expires: false,
    allow_all_services: false,
    allow_all_nodes: false,
    expires_at: expect.any(String),
  });
});
