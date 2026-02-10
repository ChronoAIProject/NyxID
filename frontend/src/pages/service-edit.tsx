import { useEffect } from "react";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import { useService, useUpdateService } from "@/hooks/use-services";
import {
  updateServiceSchema,
  type UpdateServiceFormData,
} from "@/schemas/services";
import { getAuthTypeLabel } from "@/lib/constants";
import { ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { PageHeader } from "@/components/shared/page-header";
import { IdentityPropagationConfig } from "@/components/dashboard/identity-propagation-config";
import { Separator } from "@/components/ui/separator";
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
import { Skeleton } from "@/components/ui/skeleton";
import { AlertCircle } from "lucide-react";
import { toast } from "sonner";

export function ServiceEditPage() {
  const { serviceId } = useParams({ strict: false }) as { serviceId: string };
  const navigate = useNavigate();
  const { data: service, isLoading, error } = useService(serviceId);
  const updateMutation = useUpdateService();
  const user = useAuthStore((s) => s.user);

  const form = useForm<UpdateServiceFormData>({
    resolver: zodResolver(updateServiceSchema),
    defaultValues: {
      name: "",
      description: "",
      base_url: "",
      api_spec_url: "",
      identity_propagation_mode: "none",
      identity_include_user_id: false,
      identity_include_email: false,
      identity_include_name: false,
      identity_jwt_audience: "",
    },
  });

  useEffect(() => {
    if (service) {
      form.reset({
        name: service.name,
        description: service.description ?? "",
        base_url: service.base_url,
        api_spec_url: service.api_spec_url ?? "",
        identity_propagation_mode:
          (service.identity_propagation_mode as UpdateServiceFormData["identity_propagation_mode"]) ??
          "none",
        identity_include_user_id: service.identity_include_user_id ?? false,
        identity_include_email: service.identity_include_email ?? false,
        identity_include_name: service.identity_include_name ?? false,
        identity_jwt_audience: service.identity_jwt_audience ?? "",
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [service]);

  async function onSubmit(data: UpdateServiceFormData) {
    if (!service) return;
    try {
      await updateMutation.mutateAsync({
        serviceId: service.id,
        data,
      });
      toast.success("Service updated");
      void navigate({
        to: "/services/$serviceId",
        params: { serviceId },
      });
    } catch (err) {
      if (err instanceof ApiError) {
        form.setError("root", { message: err.message });
      } else {
        toast.error("Failed to update service");
      }
    }
  }

  if (isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-64" />
        <Skeleton className="h-96 w-full" />
      </div>
    );
  }

  if (error || !service) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <AlertCircle className="mb-4 h-12 w-12 text-muted-foreground/50" />
        <h3 className="mb-2 text-lg font-semibold">Service not found</h3>
        <p className="mb-4 text-sm text-muted-foreground">
          The service you are trying to edit does not exist or has been deleted.
        </p>
        <Button
          variant="outline"
          onClick={() => void navigate({ to: "/services" })}
        >
          Back to Services
        </Button>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <PageHeader
        breadcrumbs={[
          { label: "Services", to: "/services" },
          {
            label: service.name,
            to: `/services/${serviceId}`,
          },
          { label: "Edit" },
        ]}
        title={`Edit ${service.name}`}
      />

      <div className="max-w-2xl">
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

            {user && (
              <>
                <Separator className="my-2" />
                <div className="space-y-2">
                  <h3 className="text-sm font-semibold">
                    Identity Propagation
                  </h3>
                  <p className="text-xs text-muted-foreground">
                    Configure how user identity is forwarded to this
                    downstream service during proxy requests.
                  </p>
                  <IdentityPropagationConfig
                    mode={form.watch("identity_propagation_mode") ?? "none"}
                    includeUserId={
                      form.watch("identity_include_user_id") ?? false
                    }
                    includeEmail={
                      form.watch("identity_include_email") ?? false
                    }
                    includeName={
                      form.watch("identity_include_name") ?? false
                    }
                    jwtAudience={
                      form.watch("identity_jwt_audience") ?? ""
                    }
                    onModeChange={(v) =>
                      form.setValue(
                        "identity_propagation_mode",
                        v as UpdateServiceFormData["identity_propagation_mode"],
                      )
                    }
                    onIncludeUserIdChange={(v) =>
                      form.setValue("identity_include_user_id", v)
                    }
                    onIncludeEmailChange={(v) =>
                      form.setValue("identity_include_email", v)
                    }
                    onIncludeNameChange={(v) =>
                      form.setValue("identity_include_name", v)
                    }
                    onJwtAudienceChange={(v) =>
                      form.setValue("identity_jwt_audience", v)
                    }
                  />
                </div>
              </>
            )}

            <div className="flex items-center gap-3 pt-4">
              <Button type="submit" isLoading={updateMutation.isPending}>
                Save changes
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={() =>
                  void navigate({
                    to: "/services/$serviceId",
                    params: { serviceId },
                  })
                }
              >
                Cancel
              </Button>
            </div>
          </form>
        </Form>
      </div>
    </div>
  );
}
