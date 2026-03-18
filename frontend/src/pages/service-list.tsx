import { useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  useServices,
  useCreateService,
  useDeleteService,
} from "@/hooks/use-services";
import {
  createServiceSchema,
  type CreateServiceFormData,
  AUTH_TYPES,
  SERVICE_CATEGORIES,
  SERVICE_TYPES,
} from "@/schemas/services";
import {
  AUTH_TYPE_LABELS,
  SERVICE_CATEGORY_LABELS,
  SERVICE_TYPE_LABELS,
} from "@/lib/constants";
import { ApiError } from "@/lib/api-client";
import { ServiceCard } from "@/components/dashboard/service-card";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
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
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Plus, Server } from "lucide-react";
import { toast } from "sonner";

function parseAllowedPrincipals(input?: string): string[] {
  return (input ?? "")
    .split(/[\n,]/)
    .map((principal) => principal.trim())
    .filter(Boolean);
}

export function ServiceListPage() {
  const navigate = useNavigate();
  const { data: services, isLoading } = useServices();
  const createMutation = useCreateService();
  const deleteMutation = useDeleteService();
  const [createOpen, setCreateOpen] = useState(false);
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const form = useForm<CreateServiceFormData>({
    resolver: zodResolver(createServiceSchema),
    defaultValues: {
      name: "",
      description: "",
      service_type: "http",
      base_url: "",
      auth_type: "api_key",
      service_category: "connection",
      host: "",
      port: "22",
      certificate_auth_enabled: false,
      certificate_ttl_minutes: "30",
      allowed_principals: "",
    },
  });

  const serviceType = form.watch("service_type");
  const certificateAuthEnabled = form.watch("certificate_auth_enabled") ?? false;

  async function onSubmit(data: CreateServiceFormData) {
    try {
      const created =
        data.service_type === "ssh"
          ? await createMutation.mutateAsync({
              name: data.name,
              description: data.description || undefined,
              service_type: "ssh",
              service_category: data.service_category ?? "internal",
              ssh_config: {
                host: (data.host ?? "").trim(),
                port: Number(data.port),
                certificate_auth_enabled: data.certificate_auth_enabled ?? false,
                certificate_ttl_minutes: Number(data.certificate_ttl_minutes || "30"),
                allowed_principals: parseAllowedPrincipals(data.allowed_principals),
              },
            })
          : await createMutation.mutateAsync({
              name: data.name,
              description: data.description || undefined,
              service_type: "http",
              base_url: data.base_url ?? "",
              auth_type: data.auth_type ?? "api_key",
              service_category: data.service_category ?? "connection",
            });

      toast.success("Service created successfully");
      setCreateOpen(false);
      form.reset();
      void navigate({
        to: "/services/$serviceId",
        params: { serviceId: created.id },
      });
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create service");
      }
    }
  }

  async function handleDelete(id: string) {
    setDeletingId(id);
    try {
      await deleteMutation.mutateAsync(id);
      toast.success("Service deleted successfully");
    } catch {
      toast.error("Failed to delete service");
    } finally {
      setDeletingId(null);
    }
  }

  return (
    <div className="space-y-8">
      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">Services</h2>
          <p className="text-sm text-muted-foreground">
            Manage downstream services and their authentication.
          </p>
        </div>
        <Dialog open={createOpen} onOpenChange={setCreateOpen}>
          <DialogTrigger asChild>
            <Button className="w-fit">
              <Plus className="mr-2 h-4 w-4" />
              Create Service
            </Button>
          </DialogTrigger>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Service</DialogTitle>
              <DialogDescription>
                Pick the service type first, then configure the fields NyxID should manage.
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
                  name="service_type"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Service Type</FormLabel>
                      <Select
                        value={field.value}
                        onValueChange={(value) => {
                          field.onChange(value);
                          form.clearErrors();
                          if (value === "ssh") {
                            form.setValue("service_category", "internal");
                          } else {
                            form.setValue("service_category", "connection");
                          }
                        }}
                      >
                        <FormControl>
                          <SelectTrigger>
                            <SelectValue placeholder="Select service type" />
                          </SelectTrigger>
                        </FormControl>
                        <SelectContent>
                          {SERVICE_TYPES.map((type) => (
                            <SelectItem key={type} value={type}>
                              {SERVICE_TYPE_LABELS[type] ?? type}
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
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Service Name</FormLabel>
                      <FormControl>
                        <Input placeholder="My Service" {...field} />
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
                          className="flex min-h-[60px] w-full rounded-[10px] border border-input bg-transparent px-3 py-2 text-sm ring-offset-background placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
                          placeholder="Optional description"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {serviceType === "ssh" ? (
                  <>
                    <div className="grid gap-4 sm:grid-cols-2">
                      <FormField
                        control={form.control}
                        name="host"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>SSH Host</FormLabel>
                            <FormControl>
                              <Input placeholder="ssh.internal.example" {...field} />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />

                      <FormField
                        control={form.control}
                        name="port"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>SSH Port</FormLabel>
                            <FormControl>
                              <Input type="number" min={1} max={65535} {...field} />
                            </FormControl>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    </div>

                    <div className="flex items-center justify-between rounded-[10px] border border-border p-3">
                      <Label
                        htmlFor="create-ssh-cert-auth"
                        className="text-sm font-normal"
                      >
                        Enable short-lived SSH certificates
                      </Label>
                      <Switch
                        id="create-ssh-cert-auth"
                        checked={certificateAuthEnabled}
                        onCheckedChange={(checked) =>
                          form.setValue("certificate_auth_enabled", checked)
                        }
                      />
                    </div>

                    {certificateAuthEnabled && (
                      <div className="grid gap-4 sm:grid-cols-2">
                        <FormField
                          control={form.control}
                          name="certificate_ttl_minutes"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Certificate TTL (minutes)</FormLabel>
                              <FormControl>
                                <Input type="number" min={15} max={60} {...field} />
                              </FormControl>
                              <FormMessage />
                            </FormItem>
                          )}
                        />

                        <FormField
                          control={form.control}
                          name="allowed_principals"
                          render={({ field }) => (
                            <FormItem>
                              <FormLabel>Allowed Principals</FormLabel>
                              <FormControl>
                                <Input placeholder="ubuntu, deploy" {...field} />
                              </FormControl>
                              <p className="text-xs text-muted-foreground">
                                Comma-separated SSH usernames NyxID is allowed to sign.
                              </p>
                              <FormMessage />
                            </FormItem>
                          )}
                        />
                      </div>
                    )}
                  </>
                ) : (
                  <>
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
                      name="auth_type"
                      render={({ field }) => (
                        <FormItem>
                          <FormLabel>Auth Type</FormLabel>
                          <Select
                            value={field.value}
                            onValueChange={(value) => {
                              field.onChange(value);
                              if (value === "oidc") {
                                form.setValue("service_category", "provider");
                              } else if (value === "none") {
                                form.setValue("service_category", "internal");
                              } else if (
                                form.getValues("service_category") === "provider" ||
                                form.getValues("service_category") === "internal"
                              ) {
                                form.setValue("service_category", "connection");
                              }
                            }}
                          >
                            <FormControl>
                              <SelectTrigger>
                                <SelectValue placeholder="Select auth type" />
                              </SelectTrigger>
                            </FormControl>
                            <SelectContent>
                              {AUTH_TYPES.map((type) => (
                                <SelectItem key={type} value={type}>
                                  {AUTH_TYPE_LABELS[type] ?? type}
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                          <FormMessage />
                        </FormItem>
                      )}
                    />

                    {form.watch("auth_type") !== "oidc" && form.watch("auth_type") !== "none" && (
                      <FormField
                        control={form.control}
                        name="service_category"
                        render={({ field }) => (
                          <FormItem>
                            <FormLabel>Service Category</FormLabel>
                            <Select
                              value={field.value ?? "connection"}
                              onValueChange={field.onChange}
                            >
                              <FormControl>
                                <SelectTrigger>
                                  <SelectValue placeholder="Select category" />
                                </SelectTrigger>
                              </FormControl>
                              <SelectContent>
                                {SERVICE_CATEGORIES.filter(
                                  (cat) => cat !== "provider",
                                ).map((cat) => (
                                  <SelectItem key={cat} value={cat}>
                                    {SERVICE_CATEGORY_LABELS[cat] ?? cat}
                                  </SelectItem>
                                ))}
                              </SelectContent>
                            </Select>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    )}
                  </>
                )}

                <DialogFooter>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => setCreateOpen(false)}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={createMutation.isPending}>
                    Create service
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          </DialogContent>
        </Dialog>
      </div>

      {isLoading ? (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={`svc-skel-${String(i)}`} className="h-36 w-full" />
          ))}
        </div>
      ) : !services || services.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 text-center">
          <Server className="mb-4 h-12 w-12 text-muted-foreground/50" />
          <p className="text-sm text-muted-foreground">
            No services yet. Add a service to get started.
          </p>
        </div>
      ) : (
        <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
          {services.map((service) => (
            <ServiceCard
              key={service.id}
              service={service}
              onDelete={handleDelete}
              isDeleting={deletingId === service.id}
            />
          ))}
        </div>
      )}
    </div>
  );
}
