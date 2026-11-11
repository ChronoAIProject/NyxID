import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { TariffsEditor } from "./tariffs-editor";
import type { AdminModel, Tariff } from "@/schemas/platform-models";
import { attestation, validSource } from "@/schemas/model-contracts";
const mock = vi.hoisted(() => ({
  change: vi.fn(),
  contractsError: null as Error | null,
}));
const contract = {
  version: "openai.responses.v2",
  family: "openai",
  fixture_sha256: "a".repeat(64),
  tokens: true,
  cache_read: true,
  split_reasoning: false,
  bill_tool_input: false,
  images: false,
  requests: false,
  cache_write_ttls: ["thirty_minutes", "twelve_hours"],
  input_modalities: [],
  output_modalities: [],
};
const definition = {
  version: contract.version,
  family: "openai",
  transports: ["direct.responses"],
  fixture_sha256: contract.fixture_sha256,
  flags: {
    tokens: "required",
    cache_read: "required",
    split_reasoning: "optional",
    bill_tool_input: "forbidden",
    images: "forbidden",
    requests: "optional",
  },
  cache_write_ttls: contract.cache_write_ttls,
  input_modalities: [],
  output_modalities: [],
  default_contract: contract,
};
const priceComponent = {
  key: "input",
  metric: "input_tokens",
  selector: { kind: "input" },
  credits_per_unit: "0.01",
  explicit_free: false,
  max_quantity: 100,
};
const tariff: Tariff = {
  id: "tariff",
  service_id: "billing",
  model_id: "model",
  public_model: "nyxid/test",
  sync_error: null,
  created_at: "2026-11-01T00:00:00Z",
  revision: 1,
  lane: "platform_key",
  status: "synced",
  variant: {},
  source: "historical evidence",
  contract: {
    ...contract,
    version: "openai.responses.v1",
    fixture_sha256: "b".repeat(64),
  },
  bounds: { max_input_tokens: 100 },
  components: [{ ...priceComponent, lago_metric_code: "test_metric" }],
};
const model = {
  id: "model",
  public_id: "nyxid/test",
  family: "openai",
  revision: 5,
  billing_service_id: "billing",
} as AdminModel;
vi.mock("@/hooks/use-platform-models", () => ({
  adminModelPath: (id: string) => `/admin/platform-models/${id}`,
  useModelContracts: () => ({
    data: mock.contractsError ? undefined : [definition],
    error: mock.contractsError,
  }),
  useModelTariffs: () => ({ data: [tariff] }),
  usePlatformModelChange: () => ({
    mutateAsync: mock.change,
    isPending: false,
  }),
}));
vi.mock("@/hooks/use-admin-billing", () => ({
  useAdminBillingService: () => ({}),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.contractsError = null;
});
function show(readOnly = false) {
  render(<TariffsEditor model={model} readOnly={readOnly} reload={vi.fn()} />);
}
describe("certified tariff authoring", () => {
  it("defaults to v2, requires fresh verification, and submits exact generated fields", async () => {
    const user = userEvent.setup();
    show();
    await user.click(screen.getByRole("button", { name: "Use as template" }));
    expect(screen.getByLabelText("Certified contract")).toHaveValue(
      "openai.responses.v2",
    );
    expect(screen.queryByLabelText("Contract JSON")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Bill tool input")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Images")).not.toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", { name: "Cache writes: Thirty minutes" }),
    ).toBeChecked();
    expect(
      screen.queryByLabelText("Cache writes: Five minutes"),
    ).not.toBeInTheDocument();
    const save = screen.getByRole("button", {
      name: "Create pending revision",
    });
    expect(save).toBeDisabled();
    await user.type(
      screen.getByLabelText("Verification reference (link or note)"),
      "https://provider.test/prices",
    );
    expect(save).toBeDisabled();
    await user.click(
      screen.getByRole("checkbox", {
        name: "I verified this model's limits, features and prices",
      }),
    );
    await user.click(screen.getByRole("checkbox", { name: "Split reasoning" }));
    await user.click(
      screen.getByRole("checkbox", { name: "Cache writes: Twelve hours" }),
    );
    await waitFor(() => expect(save).toBeEnabled());
    await user.click(save);
    expect(mock.change).toHaveBeenCalledWith({
      path: "/admin/platform-models/model/tariffs",
      method: "post",
      body: {
        revision: 5,
        lane: "platform_key",
        variant: {},
        bounds: tariff.bounds,
        components: [priceComponent],
        source: "contract-attested-v1:nyxid/test https://provider.test/prices",
        contract: {
          ...contract,
          split_reasoning: true,
          cache_write_ttls: ["thirty_minutes"],
        },
      },
    });
  });
  it("historical revisions remain readable and publication remains available", () => {
    show();
    expect(screen.getByText(/historical evidence/)).toHaveTextContent(
      "openai.responses.v1",
    );
    expect(
      screen.getByRole("button", { name: "Publish / retry rate sync" }),
    ).toBeEnabled();
  });
  it("read-only viewers cannot author or publish", () => {
    show(true);
    expect(
      screen.queryByRole("button", { name: "New price revision" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Use as template" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Publish / retry rate sync" }),
    ).not.toBeInTheDocument();
  });
  it("metadata failure blocks new authoring without hiding historical prices", () => {
    mock.contractsError = new Error("Contract metadata unavailable");
    show();
    expect(
      screen.getByRole("button", { name: "New price revision" }),
    ).toBeDisabled();
    expect(screen.getByText(/historical evidence/)).toBeInTheDocument();
  });
  it("rejects unbounded or control-character verification sources", () => {
    expect(validSource(attestation("nyxid/test", "verified"))).toBe(true);
    expect(validSource(attestation("nyxid/test", "é".repeat(500)))).toBe(false);
    expect(validSource(attestation("nyxid/test", "note\nsecret"))).toBe(false);
  });
});
