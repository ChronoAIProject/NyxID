import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { Form, useAppForm } from "@/components/ui/form";
import type { UpdateServiceFormData } from "@/schemas/services";
import type { DownstreamService } from "@/types/api";
import { PlatformServiceFields } from "./platform-service-fields";
vi.mock("@/components/admin-credits/credit-pickers", () => ({
  UserPicker: () => <span>Owner picker</span>,
}));
function Harness({ implicit = false }: { readonly implicit?: boolean }) {
  const form = useAppForm<UpdateServiceFormData>({
    defaultValues: { name: "Service", service_type: "http" },
  });
  return (
    <Form {...form}>
      <PlatformServiceFields
        service={{ legacy_public_master: implicit } as DownstreamService}
      />
      <button disabled={!form.formState.isDirty}>Save settings</button>
    </Form>
  );
}
it("marks platform switches dirty and preserves write-only credential entry", async () => {
  render(<Harness />);
  expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled();
  expect(screen.getByLabelText("Replace platform credential")).toHaveValue("");
  expect(screen.getByText(/No lane prices are configured/)).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("switch", { name: "Enable platform key" }),
  );
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  expect(screen.getByText("Owner picker")).toBeInTheDocument();
});
it("enabling a lane shows pricing and marks legacy billing superseded", async () => {
  render(<Harness />);
  await userEvent.click(screen.getAllByRole("switch", { name: "Free" })[0]!);
  expect(screen.getByLabelText("Credits per unit")).toBeInTheDocument();
  expect(
    screen.getByText(/Billing lanes supersede legacy platform billing/),
  ).toBeInTheDocument();
});

it("shows legacy platform keys as enabled and public until explicitly changed", async () => {
  render(<Harness implicit />);
  expect(screen.getByText("Enabled, public (implicit)")).toBeInTheDocument();
  expect(
    screen.getByRole("switch", { name: "Enable platform key" }),
  ).toBeChecked();
  expect(screen.getByRole("button", { name: "Save settings" })).toBeDisabled();
  await userEvent.click(
    screen.getByRole("switch", { name: "Enable platform key" }),
  );
  expect(
    screen.getByRole("switch", { name: "Enable platform key" }),
  ).not.toBeChecked();
  expect(
    screen.queryByText("Enabled, public (implicit)"),
  ).not.toBeInTheDocument();
});

it("marks endpoint rules dirty and can explicitly clear the policy", async () => {
  render(<Harness />);
  const user = userEvent.setup();
  await user.click(
    screen.getByRole("switch", { name: "Restrict allowed endpoints" }),
  );
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Add endpoint rule" }));
  expect(screen.getByLabelText("Rule 1 path")).toHaveValue("/");
  await user.click(
    screen.getByRole("switch", { name: "Restrict allowed endpoints" }),
  );
  expect(screen.queryByLabelText("Rule 1 path")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
});

it.each(["seeded", "private", "public"])(
  "preserves absent platform configuration when staging or rotating a %s service credential",
  async (kind) => {
    const submit = vi.fn();
    function Editor() {
      const form = useAppForm<UpdateServiceFormData>({
        defaultValues: {
          name: "Service",
          service_type: "http",
          credential: "",
        },
      });
      return (
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)}>
            <PlatformServiceFields
              service={
                {
                  provider_config_id: kind === "seeded" ? "telnyx" : null,
                  legacy_public_master: kind === "public",
                  credential_configured: kind !== "seeded",
                  visibility: kind === "private" ? "private" : "public",
                } as DownstreamService
              }
            />
            <button disabled={!form.formState.isDirty}>Save</button>
          </form>
        </Form>
      );
    }
    render(<Editor />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.type(
      screen.getByLabelText("Replace platform credential"),
      "staged-fixture",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(submit).toHaveBeenCalledOnce();
    expect(submit.mock.calls[0]?.[0]).toMatchObject({
      credential: "staged-fixture",
    });
    expect(submit.mock.calls[0]?.[0].platform_key).toBeUndefined();
  },
);

it.each([
  [undefined, "status unavailable"],
  [null, "stored but unavailable"],
  [false, "not configured"],
  [true, "configured"],
] as const)(
  "renders existing service credential status %s without guessing",
  (status, label) => {
    function StatusEditor() {
      const form = useAppForm<UpdateServiceFormData>({
        defaultValues: { name: "Service", service_type: "http" },
      });
      return (
        <Form {...form}>
          <PlatformServiceFields
            service={{ credential_configured: status } as DownstreamService}
          />
        </Form>
      );
    }
    render(<StatusEditor />);
    expect(screen.getByText(`Shared credential: ${label}`)).toBeInTheDocument();
  },
);
