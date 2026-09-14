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
function Harness() {
  const form = useAppForm<UpdateServiceFormData>({
    defaultValues: { name: "Service", service_type: "http" },
  });
  return (
    <Form {...form}>
      <PlatformServiceFields form={form} service={{} as DownstreamService} />
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
