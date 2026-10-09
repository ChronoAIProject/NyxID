const endpoints = vi.hoisted(() => ({
  data: [] as { name: string; is_active: boolean }[],
}));
vi.mock("@/hooks/use-endpoints", () => ({
  useEndpoints: () => ({ data: endpoints.data }),
}));
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { Form, useAppForm } from "@/components/ui/form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  updateServiceSchema,
  type UpdateServiceFormData,
} from "@/schemas/services";
import type { DownstreamService } from "@/types/api";
import { PlatformServiceFields } from "./platform-service-fields";
vi.mock("@/components/admin-credits/credit-pickers", () => ({
  UserPicker: () => <span>Owner picker</span>,
}));
beforeEach(() => {
  endpoints.data = [];
});
function Harness({
  implicit = false,
  xChannelBilling,
  servicePatch,
  defaults,
  onValues,
}: {
  readonly implicit?: boolean;
  readonly xChannelBilling?: DownstreamService["x_channel_billing"];
  readonly servicePatch?: Partial<DownstreamService>;
  readonly defaults?: Partial<UpdateServiceFormData>;
  readonly onValues?: (values: UpdateServiceFormData) => void;
}) {
  const form = useAppForm<UpdateServiceFormData>({
    defaultValues: { name: "Service", service_type: "http", ...defaults },
    resolver: zodResolver(updateServiceSchema),
  });
  onValues?.(form.watch());
  return (
    <Form {...form}>
      <PlatformServiceFields
        service={
          {
            legacy_public_master: implicit,
            x_channel_billing: xChannelBilling,
            ...servicePatch,
          } as DownstreamService
        }
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

it("adds and removes component prices while keeping the lane editable", async () => {
  render(<Harness />);
  const user = userEvent.setup();
  await user.click(screen.getAllByRole("switch", { name: "Free" })[0]!);
  await user.click(screen.getByRole("button", { name: "Add component" }));
  const price = screen.getByRole("textbox", { name: "Component 1 price" });
  await user.clear(price);
  await user.type(price, "0.000000250001");
  expect(price).toHaveValue("0.000000250001");
  expect(screen.getByRole("button", { name: "Save settings" })).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Remove component" }));
  expect(
    screen.queryByRole("textbox", { name: "Component 1 price" }),
  ).not.toBeInTheDocument();
  expect(screen.getByLabelText("Credits per unit")).toBeInTheDocument();
});

const declaredOperations = [
  { operation: "get_me", label: "get_me" },
  { operation: "channel_dm_send", label: "Channel direct message sent" },
];

it("lists active endpoints and declared operations, then sets and clears a price", async () => {
  endpoints.data = [
    { name: "get_me", is_active: true },
    { name: "search_recent_tweets", is_active: true },
    { name: "retired_lookup", is_active: false },
  ];
  render(
    <Harness servicePatch={{ declared_operations: declaredOperations }} />,
  );
  const user = userEvent.setup();
  await user.click(screen.getAllByRole("switch", { name: "Free" })[0]!);
  for (const label of [
    "get_me",
    "search_recent_tweets",
    "Channel direct message sent",
  ])
    expect(
      screen.getByRole("textbox", { name: `${label} operation price` }),
    ).toHaveValue("");
  expect(
    screen.queryByRole("textbox", { name: "retired_lookup operation price" }),
  ).not.toBeInTheDocument();
  const input = screen.getByRole("textbox", {
    name: "Channel direct message sent operation price",
  });
  await user.type(input, "0.25");
  expect(input).toHaveValue("0.25");
  await user.clear(input);
  expect(input).toHaveValue("");
  expect(screen.getByLabelText("Credits per unit")).toBeInTheDocument();
});

it("shows the saved sync status and keeps an unlisted saved price clearable", async () => {
  let values: UpdateServiceFormData | undefined;
  render(
    <Harness
      servicePatch={
        {
          declared_operations: declaredOperations,
          billing: {
            byok_pricing: {
              metric: "requests",
              credits_per_unit: "0.1",
              operations: [
                {
                  operation: "channel_dm_send",
                  credits_per_unit: "0.25",
                  sync_status: "synced",
                },
              ],
            },
          },
        } as Partial<DownstreamService>
      }
      defaults={{
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.1",
          operations: [
            { operation: "channel_dm_send", credits_per_unit: "0.25" },
            { operation: "removed_endpoint", credits_per_unit: "1" },
          ],
        },
      }}
      onValues={(current) => (values = current)}
    />,
  );
  expect(screen.getByText("Synced")).toBeInTheDocument();
  const stale = screen.getByRole("textbox", {
    name: "removed_endpoint operation price",
  });
  expect(stale).toHaveValue("1");
  await userEvent.clear(stale);
  expect(values?.byok_pricing?.operations).toEqual([
    { operation: "channel_dm_send", credits_per_unit: "0.25" },
  ]);
});

it("shows an invalid operation price on its row", async () => {
  render(
    <Harness servicePatch={{ declared_operations: declaredOperations }} />,
  );
  const user = userEvent.setup();
  await user.click(screen.getAllByRole("switch", { name: "Free" })[0]!);
  const input = screen.getByRole("textbox", {
    name: "Channel direct message sent operation price",
  });
  await user.type(input, "1.5x");
  expect(
    await screen.findByText(
      "Use a non-negative decimal with at most 12 decimal places",
    ),
  ).toBeInTheDocument();
  expect(input).toHaveAttribute("aria-invalid", "true");
  await user.type(input, "{backspace}");
  expect(
    screen.queryByText(
      "Use a non-negative decimal with at most 12 decimal places",
    ),
  ).not.toBeInTheDocument();
});

it("clears operation prices when the lane unit leaves Requests", async () => {
  let values: UpdateServiceFormData | undefined;
  render(
    <Harness
      servicePatch={{ declared_operations: declaredOperations }}
      defaults={{
        byok_pricing: {
          metric: "requests",
          credits_per_unit: "0.1",
          operations: [
            { operation: "channel_dm_send", credits_per_unit: "0.25" },
          ],
        },
      }}
      onValues={(current) => (values = current)}
    />,
  );
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "Unit" }));
  await user.click(screen.getByRole("option", { name: "tokens" }));
  expect(screen.queryByText("Operation prices")).not.toBeInTheDocument();
  expect(values?.byok_pricing?.operations).toEqual([]);
});

it("shows X channel billing guidance", () => {
  render(<Harness xChannelBilling={{ lane: "Your own key (BYOK)" }} />);
  expect(
    screen.getByText(/X channels bill through the Your own key/),
  ).toBeInTheDocument();
});
it("omits X channel guidance when absent", () => {
  render(<Harness />);
  expect(screen.queryByText(/X channels bill through/)).not.toBeInTheDocument();
});
