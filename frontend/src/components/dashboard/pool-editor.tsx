import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useWatch } from "react-hook-form";
import { Check } from "lucide-react";
import { toast } from "sonner";
import { firstNestedErrorMessage } from "@/lib/form-errors";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogBody,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
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
import { useCreateServicePool, useUpdateServicePool } from "@/hooks/use-pools";
import {
  createServicePoolSchema,
  defaultFailoverPolicy,
  type CreateServicePoolInput,
  type FailoverPolicy,
  type ServicePool,
} from "@/schemas/pools";
import { PoolConnectionsEditor } from "./pool-connections-editor";
import { Choice, PolicyEditor, Toggle } from "./pool-controls";
import { message, strategyLabels } from "./pool-labels";
import type { PoolOperation } from "./pool-operation-check";

export function PoolEditor({
  pool,
  orgId,
  ownerLabel = "Personal",
  onClose,
  onCloseAutoFocus,
}: {
  pool?: ServicePool;
  orgId?: string;
  ownerLabel?: string;
  onClose: () => void;
  onCloseAutoFocus?: (event: Event) => void;
}) {
  const create = useCreateServicePool();
  const update = useUpdateServicePool();
  const form = useAppForm<CreateServicePoolInput>({
    resolver: zodResolver(
      createServicePoolSchema.refine(
        (input) => Boolean(pool) || input.members.length > 0,
        {
          path: ["members"],
          message: "Add at least one connection to create a pool.",
        },
      ),
    ),
    mode: "onChange",
    defaultValues: {
      slug: pool?.slug ?? "",
      name: pool?.name ?? "",
      description: pool?.description ?? "",
      strategy: pool?.strategy ?? "priority",
      tier_balance: pool?.tier_balance ?? "round_robin",
      member_contract: pool?.member_contract ?? "same_api",
      failover: pool?.failover ?? null,
      members: pool?.members ?? [],
      is_active: pool?.is_active ?? true,
    },
  });
  const values = useWatch({ control: form.control });
  const priority = values.strategy === "priority";
  const [operation, setOperation] = useState<PoolOperation | null>(null);
  const [slugEdited, setSlugEdited] = useState(Boolean(pool));
  function setContract(contract: "same_api" | "ai_chat") {
    form.setValue("member_contract", contract);
    setOperation(null);
    if (contract === "same_api")
      form.setValue(
        "members",
        form.getValues("members").map((m) => ({ ...m, model: null })),
      );
    void form.trigger();
  }
  function setStrategy(value: string) {
    const strategy = value as CreateServicePoolInput["strategy"];
    form.setValue("strategy", strategy);
    if (strategy !== "priority") {
      setContract("same_api");
      form.setValue("tier_balance", "round_robin");
      form.setValue("failover", null);
      form.setValue(
        "members",
        form.getValues("members").map((m) => ({
          ...m,
          priority: 0,
          model: null,
          same_api_compatible: false,
        })),
      );
    }
  }
  async function save(input: CreateServicePoolInput) {
    try {
      const normalized = {
        ...input,
        members: input.members.map((m) => ({
          ...m,
          model: m.model?.trim() || null,
        })),
      };
      if (pool)
        await update.mutateAsync({
          ...normalized,
          poolId: pool.id,
          expected_revision: pool.config_revision ?? 0,
          description: input.description?.trim() || null,
        });
      else
        await create.mutateAsync({
          ...normalized,
          description: input.description?.trim() || undefined,
          org_id: orgId,
        });
      toast.success(pool ? "Service pool saved" : "Service pool created");
      onClose();
    } catch (error) {
      form.setError("root", { message: message(error) });
    }
  }
  const pending = create.isPending || update.isPending;
  const { isDirty, isValid, errors } = form.formState;
  const rootError = errors.root?.message ?? firstNestedErrorMessage(errors);
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !pending) onClose();
      }}
    >
      <DialogContent
        onCloseAutoFocus={onCloseAutoFocus}
        scrollMode="body"
        className="md:max-w-2xl [&_input:focus-visible]:border-primary [&_input:focus-visible]:ring-1 [&_input:focus-visible]:ring-primary/40"
      >
        <DialogHeader>
          <DialogTitle>
            {pool ? "Edit service pool" : "Create service pool"}
          </DialogTitle>
          <DialogDescription className="text-[12px]">
            Group your connections under one name. Owned by{" "}
            {orgId ? ownerLabel : "you"}.
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(save)} className="min-h-0 gap-4">
            <DialogBody className="space-y-5 pr-1">
              <div className="grid gap-3 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="name"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Name</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          placeholder="Reliable AI"
                          onChange={(e) => {
                            field.onChange(e);
                            if (!slugEdited)
                              form.setValue(
                                "slug",
                                e.target.value
                                  .toLowerCase()
                                  .replace(/[^a-z0-9]+/g, "-")
                                  .slice(0, 80)
                                  .replace(/^-|-$/g, ""),
                                { shouldTouch: false },
                              );
                          }}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="slug"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Pool slug</FormLabel>
                      <FormControl>
                        <Input
                          {...field}
                          placeholder="reliable-ai"
                          onChange={(e) => {
                            setSlugEdited(true);
                            field.onChange(e);
                          }}
                        />
                      </FormControl>
                      <p className="text-[11px] text-muted-foreground">
                        The stable name used by your apps and CLI.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
              <div className="space-y-2">
                <Choice
                  label="Routing"
                  value={values.strategy ?? "priority"}
                  options={Object.entries(strategyLabels)}
                  onChange={setStrategy}
                />
                <p className="text-[12px] text-muted-foreground">
                  {priority
                    ? "Try your preferred connection first, then a backup on retryable failures. A provider quota rejection (429) can fall back automatically."
                    : "Send each request to one connection. This mode does not retry on another connection."}
                </p>
              </div>
              {priority && (
                <fieldset className="space-y-2">
                  <legend className="mb-2 text-[12px] font-medium">
                    What will this pool handle?
                  </legend>
                  <div className="grid gap-2 sm:grid-cols-2">
                    {(
                      [
                        [
                          "same_api",
                          "Same API",
                          "Connections that accept the same paths and request format. Requests pass through unchanged.",
                        ],
                        [
                          "ai_chat",
                          "AI chat",
                          "Chat across supported AI providers. Choose a model for each connection; NyxID translates the requests.",
                        ],
                      ] as const
                    ).map(([value, label, description]) => (
                      <button
                        key={value}
                        type="button"
                        aria-pressed={values.member_contract === value}
                        onClick={() => setContract(value)}
                        className={`rounded-xl border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-surface ${values.member_contract === value ? "border-nyx-500/50 bg-white/[0.04]" : "border-border/50 hover:bg-white/[0.03]"}`}
                      >
                        <span className="flex items-center gap-2 text-[12px] font-medium">
                          {values.member_contract === value && (
                            <Check className="size-3 text-nyx-secondary-400" />
                          )}
                          {label}
                        </span>
                        <span className="mt-1 block text-[11px] leading-relaxed text-muted-foreground">
                          {description}
                        </span>
                      </button>
                    ))}
                  </div>
                </fieldset>
              )}
              <PoolConnectionsEditor
                form={form}
                pool={pool}
                orgId={orgId}
                operation={operation}
                setOperation={setOperation}
              />
              <details className="rounded-xl border border-border/50 p-3">
                <summary className="cursor-pointer text-[12px] font-medium">
                  Advanced settings{values.failover ? " · Custom retries" : ""}
                </summary>
                <div className="space-y-4 pt-4">
                  <FormField
                    control={form.control}
                    name="description"
                    render={({ field }) => (
                      <FormItem>
                        <FormLabel>Description (optional)</FormLabel>
                        <FormControl>
                          <Input {...field} value={field.value ?? ""} />
                        </FormControl>
                        <FormMessage />
                      </FormItem>
                    )}
                  />
                  <Toggle
                    label="Pool enabled"
                    checked={values.is_active ?? true}
                    onChange={(v) => form.setValue("is_active", v)}
                  />
                  {priority && (
                    <>
                      <Choice
                        label="Connections with the same priority"
                        value={values.tier_balance ?? "round_robin"}
                        options={[
                          ["round_robin", "Take turns"],
                          ["weighted", "Share by weight"],
                        ]}
                        onChange={(v) =>
                          form.setValue(
                            "tier_balance",
                            v as "round_robin" | "weighted",
                          )
                        }
                      />
                      <Toggle
                        label="Customize retry settings"
                        checked={values.failover != null}
                        onChange={(v) =>
                          form.setValue(
                            "failover",
                            v ? structuredClone(defaultFailoverPolicy) : null,
                          )
                        }
                      />
                      {values.failover ? (
                        <PolicyEditor
                          policy={values.failover as FailoverPolicy}
                          onChange={(v) => form.setValue("failover", v)}
                        />
                      ) : (
                        <p className="text-[12px] text-muted-foreground">
                          Up to 3 attempts, 60 seconds per attempt, and 120
                          seconds overall. Failed connections rest for 5–300
                          seconds. Replaying a possibly accepted POST is off.
                        </p>
                      )}
                    </>
                  )}
                </div>
              </details>
            </DialogBody>
            {rootError && <ErrorBanner message={rootError} />}
            <DialogFooter className="pt-3 md:pt-3">
              <Button
                type="button"
                variant="outline"
                disabled={pending}
                onClick={onClose}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                isLoading={pending}
                disabled={!isDirty || !isValid}
              >
                {pool ? "Save" : "Create pool"}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  );
}
