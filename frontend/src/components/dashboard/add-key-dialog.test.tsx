import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CatalogEntry } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
import { AddKeyDialog } from "./add-key-dialog";

const { catalog, createKeyMutate, mockNavigate, toastFns } = vi.hoisted(() => ({
  catalog: { entries: [] as unknown[] },
  createKeyMutate: vi.fn(),
  mockNavigate: vi.fn(),
  toastFns: { success: vi.fn(), error: vi.fn() },
}));

vi.mock("@/hooks/use-keys", () => ({
  useCatalog: () => ({ data: catalog.entries, isLoading: false }),
  useCreateKey: () => ({ mutate: createKeyMutate, isPending: false }),
}));

// RoutingStep reads online nodes; OwnerPicker reads admin orgs. Empty
// arrays keep the node picker empty and hide the owner picker entirely
// (it renders null without an admin org), so neither pulls in extra deps.
vi.mock("@/hooks/use-nodes", () => ({
  useNodes: () => ({ data: [], isLoading: false }),
}));
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({ data: [] }),
}));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => mockNavigate,
}));

vi.mock("sonner", () => ({ toast: toastFns }));

const OPENAI_ENTRY = {
  slug: "openai",
  name: "OpenAI",
  description: "OpenAI API",
  base_url: "https://api.openai.com/v1",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  requires_gateway_url: false,
  service_type: "http",
} as unknown as CatalogEntry;

beforeEach(() => {
  vi.clearAllMocks();
  catalog.entries = [OPENAI_ENTRY];
});

/**
 * Type into an input addressed by its DOM id. Labels here are dynamic, and
 * the dialog renders in a Radix portal under document.body (not the render
 * container), so query the whole document.
 */
async function typeInto(
  user: ReturnType<typeof userEvent.setup>,
  id: string,
  value: string,
) {
  const el = document.querySelector<HTMLInputElement>(`#${id}`);
  if (!el) throw new Error(`input #${id} not found`);
  await user.type(el, value);
}

describe("AddKeyDialog — custom endpoint path", () => {
  it("creates a key from a hand-entered endpoint and navigates to it", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-1" });
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

    // Catalog step → choose "Custom Endpoint".
    await user.click(
      screen.getByRole("button", { name: /Custom Endpoint/i }),
    );
    // Routing step → keep the default "Direct" routing.
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // Form step → fill the custom endpoint, label and credential.
    await typeInto(user, "add-key-label", "My Custom API");
    await typeInto(user, "add-key-credential", "sk-custom-123");
    await typeInto(
      user,
      "add-key-endpoint",
      "https://my.endpoint/v1",
    );

    await user.click(screen.getByRole("button", { name: "Create Service" }));

    await waitFor(() => expect(createKeyMutate).toHaveBeenCalledTimes(1));
    expect(createKeyMutate).toHaveBeenCalledWith(
      {
        credential: "sk-custom-123",
        label: "My Custom API",
        endpoint_url: "https://my.endpoint/v1",
        auth_method: "bearer",
        auth_key_name: "Authorization",
      },
      expect.anything(),
    );
    expect(toastFns.success).toHaveBeenCalledWith("Key created");
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "new-key-1" },
    });
  });

  it("surfaces the API error message when key creation fails", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onError?.(
        new ApiError(400, {
          error: "bad_request",
          error_code: 1000,
          message: "Endpoint URL is invalid",
        }),
      );
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

    await user.click(
      screen.getByRole("button", { name: /Custom Endpoint/i }),
    );
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );
    await typeInto(user, "add-key-label", "Broken");
    await typeInto(user, "add-key-credential", "sk-x");
    await typeInto(user, "add-key-endpoint", "not-a-url");
    await user.click(screen.getByRole("button", { name: "Create Service" }));

    await waitFor(() =>
      expect(toastFns.error).toHaveBeenCalledWith("Endpoint URL is invalid"),
    );
    expect(mockNavigate).not.toHaveBeenCalled();
  });
});

describe("AddKeyDialog — catalog template path", () => {
  it("creates a key from a catalog entry, omitting params that match catalog defaults", async () => {
    createKeyMutate.mockImplementation((_params, opts) => {
      opts?.onSuccess?.({ id: "new-key-2" });
    });
    const user = userEvent.setup();
    render(
      <AddKeyDialog open onOpenChange={vi.fn()} />,
    );

    // Catalog step → pick the OpenAI template (prefills label + endpoint).
    await user.click(screen.getByRole("button", { name: /OpenAI/i }));
    await user.click(
      screen.getByRole("button", { name: /Next: Enter Credentials/i }),
    );

    // Only the credential needs entering — label/endpoint are prefilled.
    await typeInto(user, "add-key-credential", "sk-openai-key");
    await user.click(screen.getByRole("button", { name: "Create Service" }));

    await waitFor(() => expect(createKeyMutate).toHaveBeenCalledTimes(1));
    // auth_method / auth_key_name are omitted because they equal the
    // catalog defaults; endpoint_url rides along from the prefilled base_url.
    expect(createKeyMutate).toHaveBeenCalledWith(
      {
        credential: "sk-openai-key",
        label: "OpenAI",
        service_slug: "openai",
        endpoint_url: "https://api.openai.com/v1",
      },
      expect.anything(),
    );
    expect(toastFns.success).toHaveBeenCalledWith("Key created");
    expect(mockNavigate).toHaveBeenCalledWith({
      to: "/keys/$keyId",
      params: { keyId: "new-key-2" },
    });
  });
});
