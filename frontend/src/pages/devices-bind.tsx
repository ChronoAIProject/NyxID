import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { CheckCircle2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { useApproveDevice } from "@/hooks/use-devices";
import { useOrgs } from "@/hooks/use-orgs";
import { ApiError } from "@/lib/api-client";
import {
  approveDeviceFormSchema,
  formatDeviceUserCodeInput,
  maskIdentifier,
  type ApproveDeviceFormData,
  type ApproveDeviceRequest,
  type ApproveDeviceResponse,
} from "@/schemas/devices";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const PERSONAL_OWNER_VALUE = "__personal__";

export function DevicesBindPage() {
  const approveDevice = useApproveDevice();
  const { data: orgs, isLoading: isOrgsLoading } = useOrgs();
  const [approvedDevice, setApprovedDevice] =
    useState<ApproveDeviceResponse | null>(null);

  const adminOrgs = (orgs ?? []).filter((org) => org.your_role === "admin");

  const form = useForm<
    ApproveDeviceFormData,
    unknown,
    ApproveDeviceRequest
  >({
    resolver: zodResolver(approveDeviceFormSchema),
    defaultValues: {
      user_code: "",
      org_id: null,
      label: "",
    },
  });

  async function handleApprove(values: ApproveDeviceRequest) {
    form.clearErrors("root");
    try {
      const response = await approveDevice.mutateAsync(values);
      setApprovedDevice(response);
      toast.success("Device approved");
    } catch (error) {
      const message = deviceApprovalErrorMessage(error);
      form.setError("root", { message });
    }
  }

  return (
    <div className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-4 py-6 sm:px-6 lg:px-8">
      <header className="flex flex-col gap-1">
        <h1 className="text-[24px] font-semibold leading-tight text-foreground sm:text-[28px]">
          Bind device
        </h1>
        <p className="text-[13px] text-muted-foreground">
          Approve a device-code request and create scoped device credentials.
        </p>
      </header>

      {approvedDevice ? (
        <ApprovalSuccess device={approvedDevice} />
      ) : (
        <Card className="rounded-lg">
          <CardHeader>
            <CardTitle>Device request</CardTitle>
            <CardDescription>
              Codes are case-insensitive and may include dashes or spaces.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <Form {...form}>
              <form
                className="flex flex-col gap-5"
                onSubmit={form.handleSubmit(handleApprove)}
              >
                {form.formState.errors.root?.message && (
                  <ErrorBanner message={form.formState.errors.root.message} />
                )}

                <FormField
                  control={form.control}
                  name="user_code"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>User code</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          autoComplete="one-time-code"
                          className="h-11 font-mono text-base"
                          inputMode="text"
                          maxLength={14}
                          placeholder="XXXX-XXXX-XXXX"
                          value={field.value ?? ""}
                          onChange={(event) =>
                            field.onChange(
                              formatDeviceUserCodeInput(event.target.value),
                            )
                          }
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="org_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Owner</FormLabel>
                      <Select
                        disabled={isOrgsLoading}
                        value={field.value ?? PERSONAL_OWNER_VALUE}
                        onValueChange={(value) =>
                          field.onChange(
                            value === PERSONAL_OWNER_VALUE ? null : value,
                          )
                        }
                      >
                        <FormControl>
                          <SelectTrigger className="h-11 text-sm">
                            <SelectValue placeholder="Personal account" />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          <SelectItem value={PERSONAL_OWNER_VALUE}>
                            Personal account
                          </SelectItem>
                          {adminOrgs.map((org) => (
                            <SelectItem key={org.id} value={org.id}>
                              {org.display_name || org.slug}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="label"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Label</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          className="h-11 text-sm"
                          maxLength={200}
                          placeholder="Hallway camera"
                          value={field.value ?? ""}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <div className="flex justify-end">
                  <Button
                    className="h-11 w-full text-sm sm:w-auto"
                    disabled={approveDevice.isPending}
                    isLoading={approveDevice.isPending}
                    type="submit"
                    variant="primary"
                  >
                    <ButtonIcon variant="primary">
                      <ShieldCheck />
                    </ButtonIcon>
                    Approve device
                  </Button>
                </div>
              </form>
            </Form>
          </CardContent>
        </Card>
      )}
    </div>
  );
}

function ApprovalSuccess({ device }: { readonly device: ApproveDeviceResponse }) {
  return (
    <Card className="rounded-lg">
      <CardHeader>
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-emerald-500/20 bg-emerald-500/10">
            <CheckCircle2 className="h-5 w-5 text-emerald-400" />
          </div>
          <div className="min-w-0">
            <CardTitle>Device approved</CardTitle>
            <CardDescription>
              Device will pick up credentials on its next poll.
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent>
        <dl className="grid gap-3 text-[13px] sm:grid-cols-2">
          <DetailRow label="Device" value={device.device_label} />
          <DetailRow label="HW ID" value={device.hw_id} />
          <DetailRow label="API key" value={maskIdentifier(device.api_key_id)} />
          <DetailRow label="Node" value={maskIdentifier(device.node_id)} />
        </dl>
      </CardContent>
    </Card>
  );
}

function DetailRow({
  label,
  value,
}: {
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="min-w-0 rounded-lg border border-border bg-background/30 px-3 py-2">
      <dt className="text-[11px] font-medium uppercase text-muted-foreground">
        {label}
      </dt>
      <dd className="mt-1 truncate font-mono text-[13px] text-foreground">
        {value}
      </dd>
    </div>
  );
}

function deviceApprovalErrorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return "Device approval failed. Try again.";
  }

  switch (error.errorCode) {
    case 9000:
    case 9003:
      return "That device code is not valid.";
    case 9001:
      return "That device code has expired.";
    case 9005:
      return "That device has already received credentials.";
    case 9006:
    case 9008:
      return "Too many attempts. Wait a moment before trying again.";
    case 9007:
      return "That device request is locked after repeated failed polls.";
    default:
      return error.message || "Device approval failed. Try again.";
  }
}
