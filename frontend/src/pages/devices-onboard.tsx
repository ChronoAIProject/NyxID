import { useEffect, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useWatch } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { CheckCircle2, Printer, QrCode, XCircle } from "lucide-react";
import QRCode from "qrcode";
import { toast } from "sonner";
import { useOnboardDevice, useRevokeOnboardDevice } from "@/hooks/use-devices";
import { useKeys } from "@/hooks/use-keys";
import { useOrgs } from "@/hooks/use-orgs";
import { ApiError } from "@/lib/api-client";
import {
  maskIdentifier,
  onboardDeviceFormSchema,
  type OnboardDeviceFormData,
  type OnboardDeviceFormValues,
  type OnboardDeviceRequest,
  type OnboardDeviceResponse,
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
  useAppForm,
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

export function DevicesOnboardPage() {
  const onboardDevice = useOnboardDevice();
  const revokeOnboardDevice = useRevokeOnboardDevice();
  const { data: orgs, isLoading: isOrgsLoading } = useOrgs();
  const { data: services, isLoading: isServicesLoading } = useKeys();
  const [onboardedDevice, setOnboardedDevice] =
    useState<OnboardDeviceResponse | null>(null);
  const qrPayload = onboardedDevice?.qr_payload ?? null;
  const qrCodeQuery = useQuery({
    queryKey: ["device-onboard-qr", qrPayload],
    queryFn: () =>
      QRCode.toDataURL(qrPayload ?? "", {
        errorCorrectionLevel: "M",
        margin: 2,
        width: 360,
      }),
    enabled: qrPayload !== null,
    staleTime: Infinity,
  });
  const qrDataUrl = qrCodeQuery.data ?? null;

  const adminOrgs = (orgs ?? []).filter((org) => org.your_role === "admin");
  const form = useAppForm<OnboardDeviceFormData, unknown, OnboardDeviceFormValues>({
    resolver: zodResolver(onboardDeviceFormSchema),
    defaultValues: {
      org_id: null,
      label: "",
      wifi_ssid: "",
      wifi_password: "",
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
    const visibleIds = new Set(grantableServices.map((service) => service.id));
    const current = form.getValues("default_services") ?? [];
    const filtered = current.filter((serviceId) => visibleIds.has(serviceId));
    if (filtered.length !== current.length) {
      // Programmatic normalization, not a user edit: don't dirty the form.
      form.setValue("default_services", filtered, {
        shouldValidate: true,
        shouldDirty: false,
        shouldTouch: false,
      });
    }
  }, [form, grantableServices]);

  async function handleOnboard(values: OnboardDeviceFormValues) {
    form.clearErrors("root");
    const request: OnboardDeviceRequest = {
      org_id: values.org_id,
      label: values.label,
      default_services: values.default_services,
    };
    try {
      const response = await onboardDevice.mutateAsync(request);
      setOnboardedDevice({
        ...response,
        qr_payload: buildFullProvisioningPayload(
          response.qr_payload,
          values.wifi_ssid,
          values.wifi_password,
        ),
      });
      toast.success("Provisioning QR generated");
    } catch (error) {
      form.setError("root", { message: deviceOnboardErrorMessage(error) });
    }
  }

  async function handleRevokeOnboard() {
    if (!onboardedDevice) return;
    try {
      await revokeOnboardDevice.mutateAsync(onboardedDevice.bootstrap_id);
      setOnboardedDevice(null);
      toast.success("Provisioning QR revoked");
    } catch (error) {
      toast.error(deviceOnboardRevokeErrorMessage(error));
    }
  }

  return (
    <>
      {onboardedDevice && qrDataUrl ? (
        <div className="hidden min-h-screen items-center justify-center bg-white p-8 print:flex">
          <img
            alt="Device onboarding QR code"
            className="h-auto w-[78vmin] max-w-[520px]"
            src={qrDataUrl}
          />
        </div>
      ) : null}

      <div
        className="mx-auto flex w-full max-w-3xl flex-col gap-5 py-6 print:hidden"
        style={{ maxWidth: "min(48rem, calc(100vw - 2rem))" }}
      >
        <header className="flex flex-col gap-1">
          <h1 className="text-[24px] font-semibold leading-tight text-foreground sm:text-[28px]">
            Onboard device
          </h1>
          <p className="break-words text-[13px] text-muted-foreground">
            Generate a one-scan provisioning QR for a headless device.
          </p>
        </header>

        {onboardedDevice ? (
        <OnboardSuccess
          device={onboardedDevice}
          qrError={qrCodeQuery.isError}
          qrDataUrl={qrDataUrl}
          isRevoking={revokeOnboardDevice.isPending}
          onPrint={() => window.print()}
          onRevoke={handleRevokeOnboard}
        />
        ) : (
          <Card className="rounded-lg">
            <CardHeader>
              <CardTitle>QR provisioning</CardTitle>
              <CardDescription>
                NyxID creates a short-lived bootstrap token. WiFi details are
                added to the QR in this browser.
              </CardDescription>
            </CardHeader>
            <CardContent>
              <Form {...form}>
                <form
                  className="flex flex-col gap-5"
                  onSubmit={form.handleSubmit(handleOnboard)}
                >
                  {form.formState.errors.root?.message && (
                    <ErrorBanner message={form.formState.errors.root.message} />
                  )}

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
                            maxLength={128}
                            placeholder="Kitchen Camera"
                            value={field.value ?? ""}
                          />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />

                  <div className="grid gap-4 sm:grid-cols-2">
                    <FormField
                      control={form.control}
                      name="wifi_ssid"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>WiFi SSID</FormLabel>
                          <FormControl>
                            <Input
                              {...field}
                              autoComplete="off"
                              className="h-11 text-sm"
                              maxLength={32}
                              placeholder="MyHomeNetwork"
                              value={field.value ?? ""}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    <FormField
                      control={form.control}
                      name="wifi_password"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>WiFi password</FormLabel>
                          <FormControl>
                            <Input
                              {...field}
                              autoComplete="off"
                              className="h-11 text-sm"
                              maxLength={63}
                              minLength={8}
                              placeholder="At least 8 characters"
                              type="password"
                              value={field.value ?? ""}
                            />
                          </FormControl>
                          <FormMessage />
                        </FormItem>
                      )}
                    />
                  </div>

                  <FormField
                    control={form.control}
                    name="default_services"
                    render={({ field }) => {
                      const selectedServices = Array.isArray(field.value)
                        ? field.value
                        : [];
                      return (
                        <FormItem>
                          <FormLabel>
                            Grant proxy access to (optional)
                          </FormLabel>
                          <FormDescription className="text-[12px] leading-relaxed">
                            Pick which of your services this device should be
                            allowed to proxy through. You can add more later
                            from the API Keys page.
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
                                const checkboxId = `device-onboard-service-${service.id}`;
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

                  <p className="rounded-lg border border-border bg-muted/25 px-3 py-2 text-[12px] leading-relaxed text-muted-foreground">
                    Your WiFi password stays in this browser. NyxID receives
                    only the owner, label, and optional service grants needed to
                    create a short-lived bootstrap token.
                  </p>

                  <div className="flex justify-end">
                    <Button
                      className="h-11 w-full text-sm sm:w-auto"
                      disabled={onboardDevice.isPending}
                      isLoading={onboardDevice.isPending}
                      type="submit"
                      variant="primary"
                    >
                      <ButtonIcon variant="primary">
                        <QrCode />
                      </ButtonIcon>
                      Generate QR
                    </Button>
                  </div>
                </form>
              </Form>
            </CardContent>
          </Card>
        )}
      </div>
    </>
  );
}

function OnboardSuccess({
  device,
  qrError,
  qrDataUrl,
  isRevoking,
  onPrint,
  onRevoke,
}: {
  readonly device: OnboardDeviceResponse;
  readonly qrError: boolean;
  readonly qrDataUrl: string | null;
  readonly isRevoking: boolean;
  readonly onPrint: () => void;
  readonly onRevoke: () => void;
}) {
  return (
    <Card className="rounded-lg">
      <CardHeader>
        <div className="flex items-center gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-lg border border-emerald-500/20 bg-emerald-500/10">
            <CheckCircle2 className="h-5 w-5 text-emerald-400" />
          </div>
          <div className="min-w-0">
            <CardTitle>Device onboarded</CardTitle>
            <CardDescription>
              Scan this QR from the headless device camera before it expires.
            </CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-5">
        <div className="flex justify-center rounded-lg border border-border bg-white p-4">
          {qrDataUrl ? (
            <img
              alt="Device onboarding QR code"
              className="h-auto w-full max-w-[360px]"
              src={qrDataUrl}
            />
          ) : (
            <div className="flex h-[360px] w-full max-w-[360px] items-center justify-center text-[13px] text-muted-foreground">
              {qrError ? "QR code rendering failed." : "Rendering QR..."}
            </div>
          )}
        </div>

        <dl className="grid gap-3 text-[13px] sm:grid-cols-2">
          <DetailRow label="Device" value={device.label} />
          <DetailRow
            label="Bootstrap"
            value={maskIdentifier(device.bootstrap_id)}
          />
          <DetailRow label="Expires" value={formatExpiry(device.expires_at)} />
        </dl>

        <div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end">
          <Button
            className="h-11 w-full text-sm sm:w-auto"
            disabled={isRevoking}
            isLoading={isRevoking}
            onClick={onRevoke}
            type="button"
            variant="destructive"
          >
            <ButtonIcon variant="destructive">
              <XCircle />
            </ButtonIcon>
            Revoke QR
          </Button>
          <Button
            className="h-11 w-full text-sm sm:w-auto"
            disabled={!qrDataUrl}
            onClick={onPrint}
            type="button"
            variant="secondary"
          >
            <ButtonIcon>
              <Printer />
            </ButtonIcon>
            Print fullscreen
          </Button>
        </div>
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

function buildFullProvisioningPayload(
  bootstrapPayload: string,
  wifiSsid: string,
  wifiPassword: string,
): string {
  const source = new URL(bootstrapPayload);
  const params = new URLSearchParams(source.search);
  params.set("ssid", wifiSsid);
  params.set("psw", wifiPassword);
  return `nyxprov://full?${params.toString()}`;
}

function formatExpiry(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString();
}

function deviceOnboardErrorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return "Device onboarding failed. Try again.";
  }

  if (error.status === 403) {
    return "You do not have permission to onboard devices for that owner.";
  }
  if (error.status === 404) {
    return "One of the selected services could not be found.";
  }

  return error.message || "Device onboarding failed. Try again.";
}

function deviceOnboardRevokeErrorMessage(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return "Device QR revocation failed. Try again.";
  }
  if (error.status === 404 || error.status === 410) {
    return "This provisioning QR is no longer active.";
  }
  return error.message || "Device QR revocation failed. Try again.";
}
