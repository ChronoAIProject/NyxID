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
} from "@/schemas/services";
import { AUTH_TYPE_LABELS, SERVICE_CATEGORY_LABELS } from "@/lib/constants";
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Plus, Server } from "lucide-react";
import { toast } from "sonner";

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
      base_url: "",
      auth_type: "api_key",
      service_category: "connection",
    },
  });

  async function onSubmit(data: CreateServiceFormData) {
    try {
      const created = await createMutation.mutateAsync(data);
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
                Register a new downstream service to connect with NyxID.
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
                          // Auto-set category based on auth type
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
