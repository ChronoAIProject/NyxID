import { render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { AdminPlatformCredentialsPage } from "./admin-platform-credentials";

vi.mock("@/hooks/use-admin-platform-credentials", () => ({
  useAdminPlatformCredentials: () => ({
    data: [
      {
        provider: "future-provider",
        label: "Future Provider",
        available: true,
        fields: [
          {
            name: "tenant",
            label: "Tenant ID",
            secret: false,
            configured: true,
            value: "tenant-42",
            help: "Your tenant",
            numeric: false,
            required: true,
          },
          {
            name: "signing_key",
            label: "Signing Key",
            secret: true,
            configured: true,
            help: "Rotate here",
            numeric: false,
            required: true,
          },
        ],
        setup_checklist: ["A provider-owned setup step"],
        callback_url: null,
        webhook_verify_token: null,
        updated_at: "today",
      },
    ],
    isLoading: false,
  }),
  useUpdatePlatformCredentials: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
  useClearPlatformCredentials: () => ({
    mutateAsync: vi.fn(),
    isPending: false,
  }),
}));

it("renders an unknown provider entirely from its descriptor with masked configured secrets", () => {
  render(<AdminPlatformCredentialsPage />);
  expect(screen.getByText("Future Provider")).toBeInTheDocument();
  expect(screen.getByLabelText("Tenant ID")).toHaveValue("tenant-42");
  expect(screen.getByLabelText("Signing Key")).toHaveAttribute(
    "type",
    "password",
  );
  expect(screen.getByLabelText("Signing Key")).toHaveValue("");
  expect(screen.getByText("A provider-owned setup step")).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Save credentials" }),
  ).toBeDisabled();
});
