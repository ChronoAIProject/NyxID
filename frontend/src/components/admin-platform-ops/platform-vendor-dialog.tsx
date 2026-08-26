import { useEffect } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { CheckCircle2, ShieldX, TriangleAlert } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
  FormSubmitErrors,
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useProvisionPlatformVendor } from "@/hooks/use-platform-ops";
import {
  platformVendorProvisionSchema,
  type PlatformVendor,
  type PlatformVendorProvision,
  type PlatformVendorRequirement,
} from "@/schemas/platform-ops";

const OPERATION_LABELS = {
  x_search: "X Search",
  speak: "Speak",
  call_and_say: "Call and Say",
} as const;

function LockedField({
  id,
  label,
  value,
}: {
  readonly id: string;
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="space-y-1.5">
      <label htmlFor={id} className="text-xs font-medium text-muted-foreground">
        {label}
      </label>
      <Input id={id} value={value} disabled className="font-mono" />
    </div>
  );
}

export function PlatformVendorDialog({
  open,
  onOpenChange,
  requirements,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly requirements: readonly PlatformVendorRequirement[];
}) {
  const provision = useProvisionPlatformVendor();
  const firstVendor = requirements[0]?.vendor ?? "twilio";
  const form = useAppForm<PlatformVendorProvision>({
    resolver: zodResolver(platformVendorProvisionSchema),
    defaultValues: { vendor: firstVendor, credential: "", note: "" },
  });

  useEffect(() => {
    if (open) {
      form.reset({ vendor: firstVendor, credential: "", note: "" });
    }
  }, [firstVendor, form, open]);

  const selectedVendor = form.watch("vendor");
  const requirement =
    requirements.find((item) => item.vendor === selectedVendor) ??
    requirements[0];
  const existingService = requirement?.existing_service ?? undefined;

  const closeDialog = () => {
    form.reset({ vendor: firstVendor, credential: "", note: "" });
    onOpenChange(false);
  };

  const onSubmit = async (data: PlatformVendorProvision) => {
    if (!requirement) return;
    try {
      await provision.mutateAsync({
        requirement,
        data,
        replaceServiceId: existingService?.id,
      });
      toast.success(
        existingService
          ? `${requirement.display_name} vendor row replaced`
          : `${requirement.display_name} vendor row added`,
      );
      closeDialog();
    } catch (error) {
      toast.error(
        error instanceof Error
          ? error.message
          : "Failed to provision platform vendor row",
      );
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (nextOpen) onOpenChange(true);
        else closeDialog();
      }}
    >
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {existingService ? "Replace platform vendor" : "Add platform vendor"}
          </DialogTitle>
          <DialogDescription>
            Create a server-owned credential row from the operation contract.
            Credentials are write-only and vendor API tools remain unpublished.
          </DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            <FormField
              control={form.control}
              name="vendor"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Vendor</FormLabel>
                  <Select
                    value={field.value}
                    onValueChange={(value) => field.onChange(value as PlatformVendor)}
                  >
                    <FormControl>
                      <SelectTrigger aria-label="Vendor">
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {requirements.map((item) => (
                        <SelectItem key={item.vendor} value={item.vendor}>
                          {item.display_name}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />

            {requirement ? (
              <>
                <div className="flex items-center justify-between gap-3 border-y border-border/50 py-2.5">
                  <div>
                    <p className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
                      Bound operation
                    </p>
                    <p className="mt-1 text-[12px] text-foreground">
                      {requirement.operation
                        ? OPERATION_LABELS[
                            requirement.operation as keyof typeof OPERATION_LABELS
                          ] ?? requirement.operation
                        : "No operation shipped yet"}
                    </p>
                  </div>
                  <Badge variant={requirement.operation ? "info" : "secondary"}>
                    {requirement.operation ?? "Pre-provisioned"}
                  </Badge>
                </div>

                <div className="grid gap-3 sm:grid-cols-2">
                  <LockedField
                    id="platform-vendor-slug"
                    label="Slug"
                    value={requirement.slug}
                  />
                  <LockedField
                    id="platform-vendor-base-url"
                    label="Base URL"
                    value={requirement.base_url}
                  />
                  <LockedField
                    id="platform-vendor-auth-method"
                    label="Auth method"
                    value={requirement.auth_method}
                  />
                  <LockedField
                    id="platform-vendor-auth-key-name"
                    label="Auth key name"
                    value={requirement.auth_key_name ?? "Not required"}
                  />
                  <LockedField
                    id="platform-vendor-category"
                    label="Category"
                    value={requirement.service_category}
                  />
                  <LockedField
                    id="platform-vendor-visibility"
                    label="Visibility"
                    value={requirement.visibility}
                  />
                </div>

                <div className="space-y-2 rounded-xl border border-border/50 bg-white/[0.02] px-4 py-3">
                  <div className="flex gap-2.5 text-[12px] text-muted-foreground">
                    <CheckCircle2 className="mt-0.5 size-3.5 shrink-0 text-success" />
                    <span>{requirement.capability_summary}</span>
                  </div>
                  <div className="flex gap-2.5 text-[12px] text-muted-foreground">
                    <ShieldX className="mt-0.5 size-3.5 shrink-0 text-warning" />
                    <span>{requirement.restriction_summary}</span>
                  </div>
                </div>

                {existingService ? (
                  <div className="flex gap-3 rounded-xl border border-warning/20 bg-warning/[0.05] px-4 py-3">
                    <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
                    <p className="text-[12px] text-muted-foreground">
                      This action deactivates <strong>{existingService.name}</strong> and
                      creates a corrected row with the same slug. No other service rows
                      are changed.
                    </p>
                  </div>
                ) : null}

                <FormField
                  control={form.control}
                  name="credential"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{requirement.credential_label}</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          type="password"
                          autoComplete="new-password"
                          spellCheck={false}
                        />
                      </FormControl>
                      <p className="text-[11px] text-muted-foreground">
                        {requirement.credential_note}
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="note"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Operator note (optional)</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          placeholder="Scope, owner, or rotation context"
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </>
            ) : null}

            <FormSubmitErrors />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={closeDialog}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={!requirement || !form.formState.isDirty || provision.isPending}
                isLoading={provision.isPending}
              >
                {existingService ? "Replace vendor row" : "Add vendor row"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
