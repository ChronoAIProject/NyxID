import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { Ban, Check, Pencil, Plus, RotateCcw } from "lucide-react";
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
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import {
  useCreatePlatformVendorTemplate,
  useDisablePlatformVendorTemplate,
  usePlatformVendorTemplates,
  useUpdatePlatformVendorTemplate,
} from "@/hooks/use-platform-ops";
import { ApiError } from "@/lib/api-client";
import {
  platformVendorTemplateFormSchema,
  type PlatformVendorRequirement,
  type PlatformVendorTemplateForm,
} from "@/schemas/platform-ops";

const OPERATION_OPTIONS = [
  { value: "x_search", label: "X Search" },
  { value: "speak", label: "Speak" },
  { value: "call_and_say", label: "Call and Say" },
] as const;

const EMPTY_FORM: PlatformVendorTemplateForm = {
  vendor: "",
  display_name: "",
  slug: "platform-",
  base_url: "https://",
  auth_method: "bearer",
  auth_key_name: null,
  credential_label: "Access token",
  credential_note: "",
  operation: null,
  capability_summary: "",
  restriction_summary: "",
  is_active: true,
};

function templateToForm(template: PlatformVendorRequirement): PlatformVendorTemplateForm {
  return {
    vendor: template.vendor,
    display_name: template.display_name,
    slug: template.slug,
    base_url: template.base_url,
    auth_method: template.auth_method,
    auth_key_name: template.auth_key_name,
    credential_label: template.credential_label,
    credential_note: template.credential_note,
    operation: template.operation,
    capability_summary: template.capability_summary,
    restriction_summary: template.restriction_summary,
    is_active: template.is_active,
  };
}

function errorMessage(error: unknown, fallback: string) {
  return error instanceof ApiError || error instanceof Error
    ? error.message
    : fallback;
}

export function PlatformVendorTemplateManager({
  open,
  onOpenChange,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const templates = usePlatformVendorTemplates();
  const create = useCreatePlatformVendorTemplate();
  const update = useUpdatePlatformVendorTemplate();
  const disable = useDisablePlatformVendorTemplate();
  const [editingId, setEditingId] = useState<string | null>(null);
  const form = useAppForm<PlatformVendorTemplateForm>({
    resolver: zodResolver(platformVendorTemplateFormSchema),
    defaultValues: EMPTY_FORM,
  });

  const editing = templates.data?.vendors.find((template) => template.id === editingId);

  useEffect(() => {
    if (open && !editingId) form.reset(EMPTY_FORM);
  }, [editingId, form, open]);

  const close = () => {
    setEditingId(null);
    form.reset(EMPTY_FORM);
    onOpenChange(false);
  };

  const startEdit = (template: PlatformVendorRequirement) => {
    setEditingId(template.id);
    form.reset(templateToForm(template));
  };

  const onSubmit = async (data: PlatformVendorTemplateForm) => {
    try {
      if (editingId) {
        await update.mutateAsync({ id: editingId, data });
        toast.success("Vendor template updated");
      } else {
        await create.mutateAsync(data);
        toast.success("Vendor template added");
      }
      setEditingId(null);
      form.reset(EMPTY_FORM);
    } catch (error) {
      toast.error(errorMessage(error, "Failed to save vendor template"));
    }
  };

  const onDisable = async (template: PlatformVendorRequirement) => {
    try {
      await disable.mutateAsync(template.id);
      toast.success(`${template.display_name} template disabled`);
      if (editingId === template.id) {
        setEditingId(null);
        form.reset(EMPTY_FORM);
      }
    } catch (error) {
      toast.error(errorMessage(error, "Failed to disable vendor template"));
    }
  };

  return (
    <Dialog open={open} onOpenChange={(nextOpen) => (nextOpen ? onOpenChange(true) : close())}>
      <DialogContent className="max-w-3xl" scrollMode="body">
        <DialogHeader>
          <DialogTitle>Vendor templates</DialogTitle>
          <DialogDescription>
            Manage the provisioning forms operators see. A template controls display
            metadata only; operation binding still enforces the server contract.
          </DialogDescription>
        </DialogHeader>

        <div className="min-h-0 flex-auto space-y-5 overflow-y-auto pr-1">
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[1.5px] text-text-tertiary">
              Current templates
            </p>
            {templates.error ? (
              <p className="text-sm text-destructive">{errorMessage(templates.error, "Failed to load vendor templates")}</p>
            ) : (
              <div className="divide-y divide-border/50 rounded-lg border border-border/60">
                {templates.data?.vendors.map((template) => (
                  <div key={template.id} className="flex items-center justify-between gap-3 px-3 py-2.5">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <span className="truncate text-sm font-medium">{template.display_name}</span>
                        <Badge variant={template.is_active ? "success" : "secondary"}>
                          {template.is_active ? "Active" : "Disabled"}
                        </Badge>
                        {template.is_seeded ? <Badge variant="secondary">Seeded</Badge> : null}
                      </div>
                      <p className="truncate font-mono text-[11px] text-muted-foreground">{template.vendor} · {template.slug}</p>
                    </div>
                    <div className="flex shrink-0 gap-1.5">
                      <Button type="button" variant="outline" size="sm" onClick={() => startEdit(template)}>
                        <Pencil className="size-3.5" />
                        Edit
                      </Button>
                      {template.is_active ? (
                        <Button type="button" variant="ghost" size="sm" onClick={() => void onDisable(template)} disabled={disable.isPending}>
                          <Ban className="size-3.5" />
                          Disable
                        </Button>
                      ) : null}
                    </div>
                  </div>
                ))}
                {!templates.isLoading && !templates.data?.vendors.length ? (
                  <p className="px-3 py-4 text-sm text-muted-foreground">No templates yet.</p>
                ) : null}
              </div>
            )}
          </div>

          <Form {...form}>
            <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
              <div className="flex items-center justify-between border-y border-border/50 py-3">
                <div>
                  <p className="text-sm font-medium">{editing ? `Edit ${editing.display_name}` : "Add a template"}</p>
                  <p className="text-xs text-muted-foreground">Use a stable `platform-` slug for rows created from this form.</p>
                </div>
                {editing ? (
                  <Button type="button" variant="ghost" size="sm" onClick={() => { setEditingId(null); form.reset(EMPTY_FORM); }}>
                    <RotateCcw className="size-3.5" />
                    New template
                  </Button>
                ) : <Plus className="size-4 text-muted-foreground" />}
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <FormField control={form.control} name="vendor" render={({ field }) => <FormItem><FormLabel>Vendor key</FormLabel><FormControl><Input {...field} placeholder="acme" /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="display_name" render={({ field }) => <FormItem><FormLabel>Display name</FormLabel><FormControl><Input {...field} placeholder="Acme Voice" /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="slug" render={({ field }) => <FormItem><FormLabel>Canonical slug</FormLabel><FormControl><Input {...field} className="font-mono" /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="base_url" render={({ field }) => <FormItem><FormLabel>Base URL</FormLabel><FormControl><Input {...field} type="url" /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="auth_method" render={({ field }) => <FormItem><FormLabel>Auth method</FormLabel><Select value={field.value} onValueChange={(value) => field.onChange(value)}><FormControl><SelectTrigger><SelectValue /></SelectTrigger></FormControl><SelectContent><SelectItem value="bearer">Bearer</SelectItem><SelectItem value="header">Header</SelectItem><SelectItem value="basic">Basic</SelectItem></SelectContent></Select><FormMessage /></FormItem>} />
                <FormField control={form.control} name="auth_key_name" render={({ field }) => <FormItem><FormLabel>Required auth key name</FormLabel><FormControl><Input value={field.value ?? ""} onChange={(event) => field.onChange(event.target.value || null)} placeholder="Optional for bearer/basic" /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="credential_label" render={({ field }) => <FormItem><FormLabel>Credential label</FormLabel><FormControl><Input {...field} /></FormControl><FormMessage /></FormItem>} />
                <FormField control={form.control} name="operation" render={({ field }) => <FormItem><FormLabel>Served operation</FormLabel><Select value={field.value ?? "none"} onValueChange={(value) => field.onChange(value === "none" ? null : value)}><FormControl><SelectTrigger><SelectValue /></SelectTrigger></FormControl><SelectContent><SelectItem value="none">No operation yet</SelectItem>{OPERATION_OPTIONS.map((option) => <SelectItem key={option.value} value={option.value}>{option.label}</SelectItem>)}</SelectContent></Select><FormMessage /></FormItem>} />
              </div>

              <FormField control={form.control} name="credential_note" render={({ field }) => <FormItem><FormLabel>Credential help text</FormLabel><FormControl><Input {...field} /></FormControl><FormMessage /></FormItem>} />
              <FormField control={form.control} name="capability_summary" render={({ field }) => <FormItem><FormLabel>Capability summary</FormLabel><FormControl><Input {...field} /></FormControl><FormMessage /></FormItem>} />
              <FormField control={form.control} name="restriction_summary" render={({ field }) => <FormItem><FormLabel>Restriction summary</FormLabel><FormControl><Input {...field} /></FormControl><FormMessage /></FormItem>} />
              <FormField control={form.control} name="is_active" render={({ field }) => <FormItem className="flex items-center justify-between rounded-lg border border-border/60 px-3 py-2"><div><FormLabel>Available to operators</FormLabel><p className="text-xs text-muted-foreground">Disabled templates stay visible here but cannot provision new rows.</p></div><FormControl><Switch checked={field.value} onCheckedChange={field.onChange} /></FormControl></FormItem>} />
              <FormSubmitErrors />
              <DialogFooter>
                <Button type="button" variant="outline" onClick={close}>Close</Button>
                <Button type="submit" variant="primary" disabled={!form.formState.isDirty || create.isPending || update.isPending} isLoading={create.isPending || update.isPending}>
                  <Check className="size-4" />
                  {editing ? "Save template" : "Add template"}
                </Button>
              </DialogFooter>
            </form>
          </Form>
        </div>
      </DialogContent>
    </Dialog>
  );
}
