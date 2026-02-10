import { useEffect } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import type { DownstreamService } from "@/types/api";
import { useUpdateService } from "@/hooks/use-services";
import {
  updateServiceSchema,
  type UpdateServiceFormData,
} from "@/schemas/services";
import { getAuthTypeLabel } from "@/lib/constants";
import { ApiError } from "@/lib/api-client";
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
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { toast } from "sonner";

interface ServiceEditDialogProps {
  readonly service: DownstreamService;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

export function ServiceEditDialog({
  service,
  open,
  onOpenChange,
}: ServiceEditDialogProps) {
  const updateMutation = useUpdateService();

  const form = useForm<UpdateServiceFormData>({
    resolver: zodResolver(updateServiceSchema),
    defaultValues: {
      name: service.name,
      description: service.description ?? "",
      base_url: service.base_url,
      api_spec_url: service.api_spec_url ?? "",
    },
  });

  useEffect(() => {
    if (open) {
      form.reset({
        name: service.name,
        description: service.description ?? "",
        base_url: service.base_url,
        api_spec_url: service.api_spec_url ?? "",
      });
    }
    // CR-15: form has a stable reference in react-hook-form; omit from deps
    // to avoid unnecessary effect re-fires
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, service]);

  async function onSubmit(data: UpdateServiceFormData) {
    try {
      await updateMutation.mutateAsync({
        serviceId: service.id,
        data,
      });
      toast.success("Service updated");
      onOpenChange(false);
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to update service");
      }
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit Service</DialogTitle>
          <DialogDescription>
            Update the configuration for {service.name}.
          </DialogDescription>
        </DialogHeader>

        <Form {...form}>
          <form
            onSubmit={form.handleSubmit(onSubmit)}
            className="space-y-4"
          >
            {form.formState.errors.root && (
              <div className="rounded-md bg-destructive/10 p-3 text-sm text-destructive">
                {form.formState.errors.root.message}
              </div>
            )}

            <FormField
              control={form.control}
              name="name"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Service Name</FormLabel>
                  <FormControl>
                    <Input {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="description"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Description</FormLabel>
                  <FormControl>
                    <textarea
                      className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                      placeholder="Optional description"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="base_url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Base URL</FormLabel>
                  <FormControl>
                    <Input
                      placeholder="https://api.example.com"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="api_spec_url"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>OpenAPI Spec URL</FormLabel>
                  <FormControl>
                    <Input
                      placeholder="https://api.example.com/openapi.json"
                      {...field}
                    />
                  </FormControl>
                  <p className="text-xs text-muted-foreground">
                    Optional. Used to auto-discover API endpoints.
                  </p>
                  <FormMessage />
                </FormItem>
              )}
            />

            <div>
              <p className="text-sm font-medium mb-1">Auth Type</p>
              <Badge variant="secondary">{getAuthTypeLabel(service)}</Badge>
              <p className="text-xs text-muted-foreground mt-1">
                Auth type cannot be changed after creation.
              </p>
            </div>

            <DialogFooter>
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button type="submit" isLoading={updateMutation.isPending}>
                Save changes
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
