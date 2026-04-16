import { useRef, useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  createApiKeySchema,
  type CreateApiKeyFormData,
  API_KEY_SCOPES,
} from "@/schemas/api-keys";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { copyToClipboard } from "@/lib/utils";
import { ApiError } from "@/lib/api-client";
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
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Plus, Copy, Check, Calendar, Shield, Server } from "lucide-react";
import { toast } from "sonner";

function toggleInArray(
  items: readonly string[],
  item: string,
): readonly string[] {
  return items.includes(item)
    ? items.filter((i) => i !== item)
    : [...items, item];
}

export function ApiKeyCreateDialog() {
  const [open, setOpen] = useState(false);
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const createMutation = useCreateApiKey();
  const expiryInputRef = useRef<HTMLInputElement | null>(null);

  const { data: services } = useKeys();
  const { data: nodes } = useNodes();

  function openDatePicker() {
    const input = expiryInputRef.current;
    if (!input) return;
    try {
      input.showPicker?.();
    } catch {
      input.focus();
    }
  }

  const form = useForm<CreateApiKeyFormData>({
    resolver: zodResolver(createApiKeySchema),
    defaultValues: {
      name: "",
      scopes: [],
      expires_at: null,
      description: null,
      allow_all_services: true,
      allow_all_nodes: true,
      allowed_service_ids: [],
      allowed_node_ids: [],
      callback_url: null,
    },
  });

  const watchAllServices = form.watch("allow_all_services") ?? true;
  const watchAllNodes = form.watch("allow_all_nodes") ?? true;

  async function onSubmit(data: CreateApiKeyFormData) {
    try {
      const result = await createMutation.mutateAsync(data);
      setCreatedKey(result.full_key);
      toast.success("API key created successfully");
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create API key");
      }
    }
  }

  async function handleCopy() {
    if (!createdKey) return;
    await copyToClipboard(createdKey);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  function handleClose() {
    setOpen(false);
    setCreatedKey(null);
    setCopied(false);
    form.reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => (o ? setOpen(true) : handleClose())}
    >
      <DialogTrigger asChild>
        <Button>
          <Plus className="mr-2 h-4 w-4" />
          Create API Key
        </Button>
      </DialogTrigger>
      <DialogContent className="max-h-[85vh] overflow-y-auto">
        {createdKey ? (
          <>
            <DialogHeader>
              <DialogTitle>API Key Created</DialogTitle>
              <DialogDescription>
                Copy your API key now. You will not be able to see it again.
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2">
              <code className="flex-1 rounded-md bg-muted p-3 font-mono text-sm break-all">
                {createdKey}
              </code>
              <Button
                variant="outline"
                size="icon"
                onClick={() => void handleCopy()}
              >
                {copied ? (
                  <Check className="h-4 w-4 text-success" />
                ) : (
                  <Copy className="h-4 w-4" />
                )}
              </Button>
            </div>
            <DialogFooter>
              <Button onClick={handleClose}>Done</Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>Create API Key</DialogTitle>
              <DialogDescription>
                Create a new API key to access the NyxID API.
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
                      <FormLabel>Name</FormLabel>
                      <FormControl>
                        <Input placeholder="My API Key" {...field} />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="scopes"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Scopes</FormLabel>
                      <div className="flex flex-wrap gap-2">
                        {API_KEY_SCOPES.map((scope) => {
                          const isSelected = (
                            field.value as readonly string[]
                          ).includes(scope);
                          return (
                            <Badge
                              key={scope}
                              variant={isSelected ? "default" : "outline"}
                              className="cursor-pointer"
                              onClick={() =>
                                field.onChange(
                                  toggleInArray(
                                    field.value as readonly string[],
                                    scope,
                                  ),
                                )
                              }
                            >
                              {scope}
                            </Badge>
                          );
                        })}
                      </div>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="expires_at"
                  render={({ field }) => {
                    const { ref: rhfRef, ...fieldRest } = field;
                    return (
                      <FormItem>
                        <FormLabel>
                          Expiry Date{" "}
                          <span className="text-muted-foreground">
                            (optional)
                          </span>
                        </FormLabel>
                        <FormControl>
                          <div className="relative">
                            <Input
                              type="date"
                              min={new Date().toISOString().slice(0, 10)}
                              className="cursor-pointer pr-10 [&::-webkit-calendar-picker-indicator]:opacity-0"
                              {...fieldRest}
                              ref={(el) => {
                                rhfRef(el);
                                expiryInputRef.current = el;
                              }}
                              value={field.value ?? ""}
                              onChange={(e) =>
                                field.onChange(e.target.value || null)
                              }
                              onClick={openDatePicker}
                              onFocus={openDatePicker}
                            />
                            <button
                              type="button"
                              aria-label="Open date picker"
                              tabIndex={-1}
                              onClick={openDatePicker}
                              className="absolute inset-y-0 right-0 flex items-center px-3 text-muted-foreground hover:text-foreground"
                            >
                              <Calendar className="h-4 w-4" />
                            </button>
                          </div>
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    );
                  }}
                />

                <FormField
                  control={form.control}
                  name="callback_url"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Callback URL{" "}
                        <span className="text-muted-foreground">
                          (optional)
                        </span>
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="url"
                          placeholder="https://my-agent.example.com/webhook"
                          {...field}
                          value={field.value ?? ""}
                          onChange={(e) =>
                            field.onChange(e.target.value || null)
                          }
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Where NyxID sends channel relay messages. Required for Channel Bot routing.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                {/* Access scope section */}
                <div className="space-y-3 rounded-lg border border-border p-4">
                  <p className="text-sm font-medium">Access Scope</p>
                  <p className="text-xs text-muted-foreground">
                    Restrict which services and nodes this key can access via proxy.
                  </p>

                  {/* Service scope */}
                  <div className="space-y-2">
                    <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                      <Shield className="h-3.5 w-3.5" />
                      Services
                    </div>
                    <FormField
                      control={form.control}
                      name="allow_all_services"
                      render={({ field }) => (
                        <FormItem>
                          <div className="flex items-center gap-2">
                            <Checkbox
                              id="allow-all-services"
                              checked={field.value}
                              onCheckedChange={(checked) =>
                                field.onChange(checked === true)
                              }
                            />
                            <Label
                              htmlFor="allow-all-services"
                              className="text-sm"
                            >
                              Allow all services
                            </Label>
                          </div>
                        </FormItem>
                      )}
                    />

                    {!watchAllServices && (
                      <FormField
                        control={form.control}
                        name="allowed_service_ids"
                        render={({ field }) => (
                          <FormItem>
                            <div className="space-y-2 rounded-md border border-border bg-muted/30 p-3">
                              <p className="text-xs text-muted-foreground">
                                Select allowed services:
                              </p>
                              {services && services.length > 0 ? (
                                services.map((s) => (
                                  <div
                                    key={s.id}
                                    className="flex items-center gap-2"
                                  >
                                    <Checkbox
                                      id={`create-svc-${s.id}`}
                                      checked={(
                                        field.value as readonly string[]
                                      ).includes(s.id)}
                                      onCheckedChange={() =>
                                        field.onChange(
                                          toggleInArray(
                                            field.value as readonly string[],
                                            s.id,
                                          ),
                                        )
                                      }
                                    />
                                    <Label
                                      htmlFor={`create-svc-${s.id}`}
                                      className="text-xs"
                                    >
                                      {s.label}
                                      <span className="ml-1 text-muted-foreground">
                                        ({s.slug})
                                      </span>
                                    </Label>
                                  </div>
                                ))
                              ) : (
                                <p className="text-xs text-muted-foreground">
                                  No external services configured yet.
                                </p>
                              )}
                            </div>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    )}
                  </div>

                  {/* Node scope */}
                  <div className="space-y-2">
                    <div className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
                      <Server className="h-3.5 w-3.5" />
                      Nodes
                    </div>
                    <FormField
                      control={form.control}
                      name="allow_all_nodes"
                      render={({ field }) => (
                        <FormItem>
                          <div className="flex items-center gap-2">
                            <Checkbox
                              id="allow-all-nodes"
                              checked={field.value}
                              onCheckedChange={(checked) =>
                                field.onChange(checked === true)
                              }
                            />
                            <Label
                              htmlFor="allow-all-nodes"
                              className="text-sm"
                            >
                              Allow all nodes
                            </Label>
                          </div>
                        </FormItem>
                      )}
                    />

                    {!watchAllNodes && (
                      <FormField
                        control={form.control}
                        name="allowed_node_ids"
                        render={({ field }) => (
                          <FormItem>
                            <div className="space-y-2 rounded-md border border-border bg-muted/30 p-3">
                              <p className="text-xs text-muted-foreground">
                                Select allowed nodes:
                              </p>
                              {nodes && nodes.length > 0 ? (
                                nodes.map((n) => (
                                  <div
                                    key={n.id}
                                    className="flex items-center gap-2"
                                  >
                                    <Checkbox
                                      id={`create-node-${n.id}`}
                                      checked={(
                                        field.value as readonly string[]
                                      ).includes(n.id)}
                                      onCheckedChange={() =>
                                        field.onChange(
                                          toggleInArray(
                                            field.value as readonly string[],
                                            n.id,
                                          ),
                                        )
                                      }
                                    />
                                    <Label
                                      htmlFor={`create-node-${n.id}`}
                                      className="text-xs"
                                    >
                                      {n.name}
                                      <Badge
                                        variant={
                                          n.status === "Online"
                                            ? "default"
                                            : "secondary"
                                        }
                                        className="ml-1 text-[10px]"
                                      >
                                        {n.status}
                                      </Badge>
                                    </Label>
                                  </div>
                                ))
                              ) : (
                                <p className="text-xs text-muted-foreground">
                                  No nodes registered yet.
                                </p>
                              )}
                            </div>
                            <FormMessage />
                          </FormItem>
                        )}
                      />
                    )}
                  </div>
                </div>

                <DialogFooter>
                  <Button type="button" variant="outline" onClick={handleClose}>
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={createMutation.isPending}>
                    Create key
                  </Button>
                </DialogFooter>
              </form>
            </Form>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
