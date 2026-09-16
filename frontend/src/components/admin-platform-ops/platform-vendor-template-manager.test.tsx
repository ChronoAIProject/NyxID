import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { PlatformVendorTemplateManager } from "./platform-vendor-template-manager";
const mock = vi.hoisted(() => ({
  update: vi.fn(),
  disable: vi.fn(),
  active: true,
}));
vi.mock("@/hooks/use-platform-ops", () => ({
  usePlatformVendorTemplates: () => ({
    data: {
      vendors: [
        {
          id: "template-a",
          vendor: "custom",
          display_name: "Custom",
          slug: "platform-custom",
          base_url: "https://example.com",
          auth_method: "bearer",
          auth_key_name: null,
          credential_label: "Key",
          credential_note: "Original note",
          operation: null,
          capability_summary: "Capability",
          restriction_summary: "Restriction",
          is_active: mock.active,
          is_seeded: false,
          existing_service: null,
          service_category: "internal",
          visibility: "public",
        },
      ],
    },
  }),
  useCreatePlatformVendorTemplate: () => ({
    isPending: false,
    mutateAsync: vi.fn(),
  }),
  useUpdatePlatformVendorTemplate: () => ({
    isPending: false,
    mutateAsync: mock.update,
  }),
  useDisablePlatformVendorTemplate: () => ({
    isPending: false,
    mutateAsync: mock.disable,
  }),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mock.active = true;
});
it("reviews and submits just the edited template note", async () => {
  const user = userEvent.setup();
  render(<PlatformVendorTemplateManager open onOpenChange={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Edit" }));
  fireEvent.change(screen.getByLabelText("Credential help text"), {
    target: { value: "Changed note" },
  });
  await user.click(screen.getByRole("button", { name: "Save template" }));
  expect(mock.update).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() =>
    expect(mock.update).toHaveBeenCalledWith({
      id: "template-a",
      data: { credential_note: "Changed note" },
    }),
  );
});
it("requires confirmation to disable a template", async () => {
  const user = userEvent.setup();
  render(<PlatformVendorTemplateManager open onOpenChange={vi.fn()} />);
  await user.click(screen.getByRole("button", { name: "Disable" }));
  expect(mock.disable).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm changes" }));
  await waitFor(() => expect(mock.disable).toHaveBeenCalledWith("template-a"));
});
