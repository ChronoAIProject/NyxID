import { useEffect, useMemo, useState } from "react";
import { useForm, useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useSearch } from "@tanstack/react-router";
import { CheckCircle2, ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { useApproveDevice } from "@/hooks/use-devices";
import { useKeys } from "@/hooks/use-keys";
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
import { Checkbox } from "@/components/ui/checkbox";
import {
  Form,
  FormControl,
  FormDescription,
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
  const search = useSearch({ strict: false }) as { user_code?: string };
  const approveDevice = useApproveDevice();
  const { data: orgs, isLoading: isOrgsLoading } = useOrgs();
  const { data: services, isLoading: isServicesLoading } = useKeys();
  const [approvedDevice, setApprovedDevice] =
    useState<ApproveDeviceResponse | null>(null);
  const initialUserCode =
    typeof search.user_code === "string"
      ? formatDeviceUserCodeInput(search.user_code)
      : "";

  const adminOrgs = (orgs ?? []).filter((org) => org.your_role === "admin");

  const form = useForm<
    ApproveDeviceFormData,
    unknown,
    ApproveDeviceRequest
  >({
    resolver: zodResolver(approveDeviceFormSchema),
    defaultValues: {
      user_code: initialUserCode,
      org_id: null,
      label: "",
      default_services: [],
    },
  });
  const selectedOwner = useWatch({
    control: form.control,
    name: "org_id",
  });
  const grantableServices = useMemo(
    () =>
      (services ?? []).filter((service) => {
        if (!service.is_active || service.auto_connected) return false;
        const source = service.credential_source;
        if (selectedOwner) {
          return (
            source?.type === "org" &&
            source.org_id === selectedOwner &&
            source.allowed
          );
        }
        return !source || source.type === "personal";
      }),
    [selectedOwner, services],
  );

  useEffect(() => {
    if (!initialUserCode || form.formState.dirtyFields.user_code) return;
    form.setValue("user_code", initialUserCode, { shouldValidate: true });
  }, [form, initialUserCode]);

  useEffect(() => {
    const visibleIds = new Set(grantableServices.map((service) => service.id));
    const current = form.getValues("default_services") ?? [];
    const filtered = current.filter((serviceId) => visibleIds.has(serviceId));
    if (filtered.length !== current.length) {
      form.setValue("default_services", filtered, { shouldValidate: true });
    }
  }, [form, grantableServices]);

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
    <div
      className="mx-auto flex w-full max-w-3xl flex-col gap-5 py-6"
      style={{ maxWidth: "min(48rem, calc(100vw - 2rem))" }}
    >
      <header className="flex flex-col gap-1">
        <h1 className="text-[24px] font-semibold leading-tight text-foreground sm:text-[28px]">
          Bind device
        </h1>
        <p className="break-words text-[13px] text-muted-foreground">
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

                <FormField
                  control={form.control}
                  name="default_services"
                  render={({ field }) => {
                    const selectedServices = Array.isArray(field.value)
                      ? field.value
                      : [];
                    return (
                      <FormItem>
                        <FormLabel>Grant proxy access to (optional)</FormLabel>
                        <FormDescription className="text-[12px] leading-relaxed">
                          Pick which of your services this device should be
                          allowed to proxy through. You can add more later from
                          the API Keys page.
                        </FormDescription>
                        <div className="max-h-56 space-y-1 overflow-y-auto rounded-lg border border-border bg-muted/25 p-2">
                          {isServicesLoading ? (
                            <p className="px-2 py-3 text-[12px] text-muted-foreground">
                              Loading services...
                            </p>
                          ) : grantableServices.length === 0 ? (
                            <p className="px-2 py-3 text-[12px] text-muted-foreground">
                              {selectedOwner
                                ? "This org has no services available for device access."
                                : "Your personal account has no services available for device access."}
                            </p>
                          ) : (
                            grantableServices.map((service) => {
                              const checkboxId = `device-bind-service-${service.id}`;
                              const checked = selectedServices.includes(
                                service.id,
                              );
                              return (
                                <div
                                  key={service.id}
                                  className="flex min-h-11 items-start gap-3 rounded-md px-2 py-2 hover:bg-accent/40"
                                >
                                  <Checkbox
                                    id={checkboxId}
                                    checked={checked}
                                    className="mt-0.5"
                                    onCheckedChange={() =>
                                      field.onChange(
                                        toggleStringArray(
                                          selectedServices,
                                          service.id,
                                        ),
                                      )
                                    }
                                  />
                                  <label
                                    htmlFor={checkboxId}
                                    className="min-w-0 flex-1 cursor-pointer"
                                  >
                                    <span className="block truncate text-[13px] font-medium text-foreground">
                                      {service.label}
                                    </span>
                                    <span className="block truncate font-mono text-[12px] text-muted-foreground">
                                      {service.slug}
                                    </span>
                                  </label>
                                </div>
                              );
                            })
                          )}
                        </div>
                        <FormMessage />
                      </FormItem>
                    );
                  }}
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

function toggleStringArray(values: readonly string[], value: string): string[] {
  return values.includes(value)
    ? values.filter((item) => item !== value)
    : [...values, value];
}

function deviceApprovalErrorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return "Device approval failed. Try again.";
  }

  switch (error.errorCode) {
    case 9500:
    case 9503:
      return "That device code is not valid.";
    case 9501:
      return "That device code has expired.";
    case 9505:
      return "That device has already received credentials.";
    case 9506:
    case 9508:
      return "Too many attempts. Wait a moment before trying again.";
    case 9507:
      return "That device request is locked after repeated failed polls.";
    default:
      return error.message || "Device approval failed. Try again.";
  }
}
