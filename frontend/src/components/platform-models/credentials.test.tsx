import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { useAuthStore } from "@/stores/auth-store";
import { ApiError } from "@/lib/api-client";
import { ProviderCredentials } from "./credentials";
import { ModelFormEditor } from "./model-form";
import { credentialError } from "@/schemas/model-credentials";
import type { ReactNode } from "react";
const mock = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn(), put: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: mock,
}));
const id = "00000000-0000-4000-8000-000000000001";
const record = {
  id,
  name: "OpenAI production",
  kind: "openai",
  enabled: true,
  created_at: "2026-11-01T00:00:00Z",
  updated_at: "2026-11-02T00:00:00Z",
  references: [],
};
const model = {
  id,
  public_id: "nyxid/gemini-test",
  display_name: "Gemini test",
  description: "",
  family: "gemini",
  capabilities: {
    protocols: [],
    input_modalities: ["text"],
    output_modalities: ["text"],
  },
  lifecycle: "draft",
  enabled: false,
  revision: 1,
  authorization_revision: 1,
  routing_revision_id: null,
  billing_service_id: null,
  entitlements: { all_people: false, rules: [] },
  routes: [],
  execution_ready: false,
  blocking_reason: "",
  required_readiness: [],
  created_at: record.created_at,
  updated_at: record.updated_at,
};
function show(node: ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return {
    client,
    ...render(
      <QueryClientProvider client={client}>{node}</QueryClientProvider>,
    ),
  };
}
beforeEach(async () => {
  await appI18n.changeLanguage("en");
  vi.clearAllMocks();
  useAuthStore.setState({
    user: {
      id,
      role: "admin",
      is_admin: true,
      capabilities: { enabled_features: ["inference:platform-models"] },
    } as never,
  });
  mock.get.mockResolvedValue({ records: [record], next_after: null });
  mock.post.mockResolvedValue(record);
  mock.put.mockResolvedValue(record);
});
afterEach(async () => {
  cleanup();
  await appI18n.changeLanguage("en");
});
describe("provider credentials", () => {
  it("switches all credential controls and save errors without changing the secret or request kind", async () => {
    show(<ProviderCredentials readOnly={false} />);
    await userEvent.click(screen.getByRole("button", { name: "Add credential" }));
    await userEvent.type(screen.getByLabelText("Credential name"), "Authored name");
    await userEvent.selectOptions(screen.getByLabelText("Credential kind"), "openai");
    const input = screen.getByLabelText("Provider credential");
    await userEvent.type(input, "private-test-only-key");
    for (const [language, label, save] of [
      ["zh-CN", "提供商凭据", "保存凭据"],
      ["zh-TW", "供應商憑證", "儲存憑證"],
    ] as const) {
      await act(async () => { await appI18n.changeLanguage(language); });
      expect(screen.getByLabelText(label)).toBe(input);
      expect(input).toHaveValue("private-test-only-key");
      expect(screen.getByRole("button", { name: save })).toBeEnabled();
    }
    mock.post.mockRejectedValue(new ApiError(400, { message: "private-test-only-key" } as never));
    await userEvent.click(screen.getByRole("button", { name: "儲存憑證" }));
    expect(await screen.findByText(/無法儲存憑證/)).toBeInTheDocument();
    expect(input).toHaveValue("");
    expect(mock.post).toHaveBeenCalledWith("/admin/platform-models/credentials", {
      name: "Authored name", kind: "openai", credential: "private-test-only-key",
    });
    await act(async () => { await appI18n.changeLanguage("zh-CN"); });
    expect(screen.getByText(/无法保存凭据/)).toBeInTheDocument();
    expect(screen.queryByText("private-test-only-key")).not.toBeInTheDocument();
  });
  it("lists metadata and references while hiding writes for read-only viewers", async () => {
    mock.get.mockResolvedValue({
      records: [
        {
          ...record,
          references: [
            {
              model_id: id,
              public_id: "nyxid/gpt-test",
              display_name: "GPT test",
              route_id: "route-one",
              enabled: true,
            },
          ],
        },
      ],
      next_after: null,
    });
    show(<ProviderCredentials readOnly />);
    expect(await screen.findByText("OpenAI production")).toBeInTheDocument();
    expect(screen.getByText(/GPT test \/ route route-one/)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Add credential" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Rotate / replace" }),
    ).not.toBeInTheDocument();
  });
  it("masks keys and keeps secret submissions out of the mutation cache and rendered errors", async () => {
    const { client } = show(<ProviderCredentials readOnly={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Add credential" }),
    );
    await userEvent.type(
      screen.getByLabelText("Credential name"),
      "OpenAI production",
    );
    await userEvent.selectOptions(
      screen.getByLabelText("Credential kind"),
      "openai",
    );
    const input = screen.getByLabelText("Provider credential");
    expect(input).toHaveAttribute("type", "password");
    await userEvent.type(input, "private-test-only-key");
    mock.post.mockRejectedValue(
      new ApiError(400, { message: "private-test-only-key" } as never),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Save credential" }),
    );
    await screen.findByText(/Could not save the credential/);
    expect(mock.post).toHaveBeenCalledWith(
      "/admin/platform-models/credentials",
      {
        name: "OpenAI production",
        kind: "openai",
        credential: "private-test-only-key",
      },
    );
    expect(screen.queryByText("private-test-only-key")).not.toBeInTheDocument();
    expect(input).toHaveValue("");
    expect(client.getMutationCache().getAll()).toHaveLength(0);
  });
  it("validates selected JSON before save and refuses external sources", async () => {
    show(<ProviderCredentials readOnly={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Add credential" }),
    );
    await userEvent.type(screen.getByLabelText("Credential name"), "Vertex");
    const file = new File(
      ['{"type":"external_account","credential_source":{"file":"secret"}}'],
      "key.json",
      { type: "application/json" },
    );
    Object.defineProperty(file, "text", {
      value: async () =>
        '{"type":"external_account","credential_source":{"file":"secret"}}',
    });
    await userEvent.upload(
      screen.getByLabelText("Service account JSON file"),
      file,
    );
    await screen.findByText(/External-account credentials are not supported/);
    expect(mock.post).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Service account JSON (paste)")).toHaveValue(
      "",
    );
  });
  it("loads a valid JSON file into the masked field and submits it without a preview", async () => {
    const raw = JSON.stringify({
      type: "service_account",
      project_id: "private-project",
      private_key_id: "fixture",
      private_key:
        "-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----",
      client_email: "fixture@private-project.iam.gserviceaccount.com",
      client_id: "fixture",
    });
    show(<ProviderCredentials readOnly={false} />);
    await userEvent.click(
      screen.getByRole("button", { name: "Add credential" }),
    );
    await userEvent.type(
      screen.getByLabelText("Credential name"),
      "Vertex production",
    );
    const file = new File([raw], "key.json", { type: "application/json" });
    Object.defineProperty(file, "text", { value: async () => raw });
    await userEvent.upload(
      screen.getByLabelText("Service account JSON file"),
      file,
    );
    const input = screen.getByLabelText("Service account JSON (paste)");
    await waitFor(() => expect(input).toHaveValue(raw));
    expect(input).toHaveAttribute("type", "password");
    expect(screen.queryByText("private-project")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Save credential" }),
    );
    await waitFor(() =>
      expect(mock.post).toHaveBeenCalledWith(
        "/admin/platform-models/credentials",
        {
          name: "Vertex production",
          kind: "gcp-service-account",
          credential: raw,
        },
      ),
    );
    await waitFor(() =>
      expect(
        screen.queryByLabelText("Service account JSON (paste)"),
      ).not.toBeInTheDocument(),
    );
  });
  it("rotates with the displayed revision and confirms disabling", async () => {
    show(<ProviderCredentials readOnly={false} />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Rotate / replace" }),
    );
    await userEvent.type(
      screen.getByLabelText("Provider credential"),
      "replacement-test-key",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Replace credential" }),
    );
    await waitFor(() =>
      expect(mock.put).toHaveBeenCalledWith(
        `/admin/platform-models/credentials/${id}`,
        {
          credential: "replacement-test-key",
          expected_updated_at: record.updated_at,
        },
      ),
    );
    await waitFor(() =>
      expect(
        screen.queryByLabelText("Provider credential"),
      ).not.toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Disable" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Confirm disable" }),
    );
    await waitFor(() =>
      expect(mock.put).toHaveBeenCalledWith(
        `/admin/platform-models/credentials/${id}`,
        { enabled: false, expected_updated_at: record.updated_at },
      ),
    );
  });
  it("does not offer disable while enabled routes reference the credential", async () => {
    mock.get.mockResolvedValue({
      records: [
        {
          ...record,
          references: [
            {
              model_id: id,
              public_id: "nyxid/test",
              display_name: "Test",
              route_id: "r",
              enabled: true,
            },
          ],
        },
      ],
      next_after: null,
    });
    show(<ProviderCredentials readOnly={false} />);
    expect(
      await screen.findByRole("button", { name: "Disable" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/Disable the referencing routes/),
    ).toBeInTheDocument();
  });
});
describe("service account validation", () => {
  const valid = {
    type: "service_account",
    project_id: "project",
    private_key_id: "id",
    private_key:
      "-----BEGIN PRIVATE KEY-----\nfixture\n-----END PRIVATE KEY-----",
    client_email: "test@project.iam.gserviceaccount.com",
    client_id: "123",
  };
  it("accepts the standard shape and rejects unknown fields and nonstandard authorities", () => {
    expect(
      credentialError(JSON.stringify(valid), "gcp-service-account"),
    ).toBeUndefined();
    for (const patch of [
      { credential_source: {} },
      { type: "external_account" },
      { token_uri: "https://example.test" },
      { universe_domain: "example.test" },
      { client_email: "bad" },
      { client_id: "" },
      { private_key: "invalid" },
    ])
      expect(
        credentialError(
          JSON.stringify({ ...valid, ...patch }),
          "gcp-service-account",
        ),
      ).toContain("service_account");
    expect(credentialError("{", "gcp-service-account")).toContain(
      "service_account",
    );
    expect(credentialError("x".repeat(65537), "openai")).toContain("64 KiB");
  });
});
describe("automatic billing identity", () => {
  async function fill() {
    await userEvent.type(
      screen.getByLabelText("Public model ID"),
      "gemini-test",
    );
    await userEvent.type(screen.getByLabelText("Display name"), "Gemini test");
  }
  it("creates draft then offering then links through the revision-fenced update", async () => {
    const onSaved = vi.fn();
    mock.post.mockImplementation(async (path: string) =>
      path.endsWith("offerings") ? record : model,
    );
    mock.put.mockResolvedValue({
      ...model,
      revision: 2,
      billing_service_id: id,
    });
    show(<ModelFormEditor readOnly={false} onSaved={onSaved} />);
    await fill();
    expect(
      screen.getByRole("checkbox", {
        name: "Create billing identity automatically",
      }),
    ).toBeChecked();
    await userEvent.click(
      screen.getByRole("button", { name: "Create disabled draft" }),
    );
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(mock.post.mock.calls.map((c) => c[0])).toEqual([
      "/admin/platform-models",
      "/admin/platform-models/offerings",
    ]);
    expect(mock.put).toHaveBeenCalledWith(
      `/admin/platform-models/${id}`,
      expect.objectContaining({
        expected_revision: 1,
        billing_service_id: id,
        enabled: false,
      }),
    );
  });
  it("retries a failed link without creating a second draft or identity", async () => {
    mock.post.mockImplementation(async (path: string) =>
      path.endsWith("offerings") ? record : model,
    );
    mock.put
      .mockRejectedValueOnce(new Error("link failed"))
      .mockResolvedValue({ ...model, revision: 2, billing_service_id: id });
    const onSaved = vi.fn();
    show(<ModelFormEditor readOnly={false} onSaved={onSaved} />);
    await fill();
    await userEvent.click(
      screen.getByRole("button", { name: "Create disabled draft" }),
    );
    await screen.findByText(/Its billing identity was also created/);
    await userEvent.click(
      screen.getByRole("button", { name: "Create disabled draft" }),
    );
    await waitFor(() => expect(onSaved).toHaveBeenCalled());
    expect(mock.post).toHaveBeenCalledTimes(2);
    expect(mock.put).toHaveBeenCalledTimes(2);
  });
  it("blocks blind create retries when the offering response is uncertain", async () => {
    mock.post.mockImplementation(async (path: string) => {
      if (path.endsWith("offerings")) throw new Error("connection lost");
      return model;
    });
    show(<ModelFormEditor readOnly={false} />);
    await fill();
    await userEvent.click(
      screen.getByRole("button", { name: "Create disabled draft" }),
    );
    await screen.findByText(/The create response was not confirmed/);
    expect(
      screen.getByRole("button", { name: "Create disabled draft" }),
    ).toBeDisabled();
    expect(mock.post).toHaveBeenCalledTimes(2);
    expect(mock.put).not.toHaveBeenCalled();
    expect(
      screen.getByText(/The disabled draft nyxid\/gemini-test already exists/),
    ).toBeInTheDocument();
  });
  it.each(["draft", "offering"])(
    "blocks blind retries after an unreadable successful %s response",
    async (stage) => {
      mock.post.mockImplementation(async (path: string) =>
        stage === "draft" || path.endsWith("offerings")
          ? { unexpected: "response" }
          : model,
      );
      const onSaved = vi.fn();
      show(<ModelFormEditor readOnly={false} onSaved={onSaved} />);
      await fill();
      await userEvent.click(
        screen.getByRole("button", { name: "Create disabled draft" }),
      );
      await screen.findByText(/The create response was not confirmed/);
      const submit = screen.getByRole("button", {
        name: "Create disabled draft",
      });
      expect(submit).toBeDisabled();
      await userEvent.click(submit);
      expect(mock.post).toHaveBeenCalledTimes(stage === "draft" ? 1 : 2);
      expect(mock.put).not.toHaveBeenCalled();
      expect(onSaved).not.toHaveBeenCalled();
    },
  );
  it("keeps a manual offering picker and skips automatic creation", async () => {
    mock.post.mockResolvedValue(model);
    mock.put.mockResolvedValue({ ...model, billing_service_id: id });
    show(<ModelFormEditor readOnly={false} />);
    await fill();
    await userEvent.click(
      screen.getByRole("checkbox", {
        name: "Create billing identity automatically",
      }),
    );
    await userEvent.selectOptions(
      await screen.findByLabelText("Billing identity"),
      id,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Create disabled draft" }),
    );
    await waitFor(() => expect(mock.put).toHaveBeenCalled());
    expect(mock.post).toHaveBeenCalledTimes(1);
  });
});
