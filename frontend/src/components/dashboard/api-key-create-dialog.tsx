import { useEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import {
  createApiKeySchema,
  type CreateApiKeyFormData,
} from "@/schemas/api-keys";
import { PLATFORM_OPTIONS } from "@/schemas/agent-bindings";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { useOrgs } from "@/hooks/use-orgs";
import { OrgScopeSelect } from "@/components/shared/org-scope-select";
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
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { AddCtaButton } from "@/components/shared/add-cta-button";
import { Badge } from "@/components/ui/badge";
import { Copy, Check } from "lucide-react";
import { PlatformIcon } from "@/components/platform-icon";
import {
  ApiKeyNameField,
  ApiKeyScopesField,
  ApiKeyExpiryField,
  ApiKeyResourceFields,
} from "./api-key-form-fields";
import { toast } from "sonner";
import type { CredentialSource } from "@/schemas/orgs";
import type { KeyInfo } from "@/types/keys";
import type { NodeInfo } from "@/types/nodes";

function sourceMatchesSelectedOwner(
  source: CredentialSource | undefined,
  targetOrgId: string | undefined,
): boolean {
  if (targetOrgId) {
    return source?.type === "org" && source.org_id === targetOrgId;
  }
  if (!source || source.type === "personal") return true;
  return source.allowed;
}

function serviceCanBeScopedToKey(
  service: KeyInfo,
  targetOrgId: string | undefined,
): boolean {
  if (service.auto_connected || !service.is_active) return false;
  return sourceMatchesSelectedOwner(service.credential_source, targetOrgId);
}

function nodeCanBeScopedToKey(
  node: NodeInfo,
  targetOrgId: string | undefined,
): boolean {
  if (targetOrgId) {
    return node.owner.kind === "org" && node.owner.id === targetOrgId;
  }
  return node.owner.kind === "user" || node.owner.kind === "org";
}

export function ApiKeyCreateDialog({
  externalOpen,
  onExternalOpenChange,
  hideTrigger,
  setupMode = false,
  initialServiceId,
}: {
  readonly externalOpen?: boolean;
  readonly onExternalOpenChange?: (open: boolean) => void;
  readonly hideTrigger?: boolean;
  readonly setupMode?: boolean;
  readonly initialServiceId?: string | null;
} = {}) {
  const [internalOpen, setInternalOpen] = useState(false);
  const open = externalOpen ?? internalOpen;
  const setOpen = onExternalOpenChange ?? setInternalOpen;
  const [createdKey, setCreatedKey] = useState<string | null>(null);
  const [createdKeyId, setCreatedKeyId] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const createMutation = useCreateApiKey();

  const { data: services } = useKeys();
  const { data: nodes } = useNodes();
  const { data: orgs } = useOrgs();

  // Only admin orgs are valid ownership targets -- members/viewers cannot
  // create org-owned keys. The selector also includes a "Personal" option
  // that maps to an undefined `target_org_id` (the default).
  const adminOrgs = useMemo(
    () => (orgs ?? []).filter((o) => o.your_role === "admin"),
    [orgs],
  );

  const form = useAppForm<CreateApiKeyFormData>({
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
      target_org_id: undefined,
    },
  });

  const watchAllServices = form.watch("allow_all_services") ?? true;
  const watchAllowedServices = form.watch("allowed_service_ids") ?? [];
  const watchTargetOrg = form.watch("target_org_id");
  const setupRequiresServiceSelection =
    setupMode && !watchAllServices && watchAllowedServices.length === 0;
  const initializedSetupRef = useRef(false);

  useEffect(() => {
    if (!open || !setupMode) {
      initializedSetupRef.current = false;
      return;
    }
    if (initializedSetupRef.current) return;
    if (initialServiceId && services === undefined) return;

    initializedSetupRef.current = true;
    const initialService = (services ?? []).find(
      (service) => service.id === initialServiceId,
    );
    const serviceSource = initialService?.credential_source;
    const targetOrgId =
      serviceSource?.type === "org" &&
      (serviceSource.role === "admin" || serviceSource.role === "owner")
        ? serviceSource.org_id
        : undefined;
    form.reset({
      name: "Claude Code Agent",
      scopes: ["proxy", "services:read"],
      expires_at: null,
      description: initialService
        ? `Isolated Agent Key for ${initialService.label}`
        : "Isolated Agent Key",
      allow_all_services: false,
      allow_all_nodes: true,
      allowed_service_ids: initialService ? [initialService.id] : [],
      allowed_node_ids: [],
      callback_url: null,
      platform: "claude-code",
      target_org_id: targetOrgId,
    });
  }, [form, initialServiceId, open, services, setupMode]);

  async function onSubmit(data: CreateApiKeyFormData) {
    try {
      const result = await createMutation.mutateAsync(data);
      setCreatedKey(result.full_key);
      setCreatedKeyId(result.id);
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
    setCreatedKeyId(null);
    setCopied(false);
    form.reset();
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => (o ? setOpen(true) : handleClose())}
    >
      {!hideTrigger && (
        <DialogTrigger asChild>
          <AddCtaButton label="Create API Key" onClick={() => {}} />
        </DialogTrigger>
      )}
      <DialogContent>
        {createdKey ? (
          <>
            <DialogHeader>
              <DialogTitle>
                {setupMode ? "Agent Key Created" : "API Key Created"}
              </DialogTitle>
              <DialogDescription>
                Copy your API key now. You will not be able to see it again.
                {setupMode
                  ? " Open the Agent Key detail page next to review service scope, add credential bindings only when this agent needs overrides, and verify proxy access."
                  : ""}
              </DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2">
              <code className="flex-1 rounded-lg bg-muted p-3 font-mono text-[12px] break-all">
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
              {setupMode && createdKeyId && (
                <Button variant="outline" asChild>
                  <Link
                    to="/keys/api-key/$keyId"
                    params={{ keyId: createdKeyId }}
                    onClick={handleClose}
                  >
                    Review Scope & Bindings
                  </Link>
                </Button>
              )}
              <Button variant="primary" onClick={handleClose}>
                Done
              </Button>
            </DialogFooter>
          </>
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>
                {setupMode ? "Create Agent Key" : "Create API Key"}
              </DialogTitle>
              <DialogDescription>
                {setupMode
                  ? "Create a scoped Agent Key using the existing key creation and service-scope controls."
                  : "Create a new API key to access the NyxID API."}
              </DialogDescription>
            </DialogHeader>

            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-[12px] text-destructive">
                    {form.formState.errors.root.message}
                  </div>
                )}

                <ApiKeyNameField form={form} />

                {setupMode && (
                  <FormField
                    control={form.control}
                    name="platform"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Platform</FormLabel>
                        <div className="flex flex-wrap gap-2">
                          {PLATFORM_OPTIONS.map((platform) => (
                            <Badge
                              key={platform}
                              variant={
                                field.value === platform
                                  ? "default"
                                  : "secondary"
                              }
                              className="cursor-pointer gap-1"
                              onClick={() => field.onChange(platform)}
                            >
                              <PlatformIcon platform={platform} size="xs" />
                              {platform}
                            </Badge>
                          ))}
                        </div>
                        <p className="text-xs text-muted-foreground">
                          Stored on the key for audit attribution. You can
                          change it later on the Agent Key detail page.
                        </p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                )}

                {adminOrgs.length > 0 && (
                  <FormField
                    control={form.control}
                    name="target_org_id"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Owner</FormLabel>
                        <FormControl>
                          <OrgScopeSelect
                            value={field.value ?? null}
                            onChange={(next) => {
                              field.onChange(next ?? undefined);
                              // Reset service scope selections when owner
                              // changes so stale selections do not
                              // round-trip to the backend.
                              form.setValue("allowed_service_ids", []);
                              form.setValue("allowed_node_ids", []);
                              form.setValue("allow_all_services", true);
                              form.setValue("allow_all_nodes", true);
                            }}
                            label="Owner"
                          />
                        </FormControl>
                        <p className="text-xs text-muted-foreground">
                          Org-owned keys are shared with every admin of that
                          organization and can only scope to services owned by
                          the same org.
                        </p>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                )}

                <ApiKeyScopesField form={form} />

                <ApiKeyExpiryField form={form} />

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
                        Where NyxID sends channel relay messages. Required for
                        Channel Bot routing.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <ApiKeyResourceFields
                  form={form}
                  services={(services ?? [])
                    .filter((service) =>
                      serviceCanBeScopedToKey(service, watchTargetOrg),
                    )
                    .map((service) => ({
                      id: service.id,
                      name: service.label || service.slug,
                    }))}
                  nodes={(nodes ?? [])
                    .filter((node) =>
                      nodeCanBeScopedToKey(node, watchTargetOrg),
                    )
                    .map((node) => ({ id: node.id, name: node.name }))}
                />

                <DialogFooter>
                  <Button type="button" variant="outline" onClick={handleClose}>
                    Cancel
                  </Button>
                  <Button
                    variant="primary"
                    type="submit"
                    isLoading={createMutation.isPending}
                    disabled={
                      !form.watch("name").trim() ||
                      setupRequiresServiceSelection
                    }
                  >
                    {setupMode ? "Create Agent Key" : "Create Key"}
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
