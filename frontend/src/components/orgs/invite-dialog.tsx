import { useState } from "react";
import { useForm } from "react-hook-form";
import { Check, Copy } from "lucide-react";
import { toast } from "sonner";
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
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ApiError } from "@/lib/api-client";
import { buildOrgInviteJoinUrl } from "@/lib/org-invite-links";
import { copyToClipboard } from "@/lib/utils";
import { useCreateInvite } from "@/hooks/use-org-invites";
import {
  createInviteRequestSchema,
  ORG_ROLES,
  type InviteResponse,
  type OrgRole,
} from "@/schemas/orgs";

interface InviteDialogProps {
  readonly orgId: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}

interface InviteFormValues {
  readonly role: OrgRole;
  readonly ttl_hours: string;
}

export function InviteDialog({ orgId, open, onOpenChange }: InviteDialogProps) {
  const createMutation = useCreateInvite();
  const [createdInvite, setCreatedInvite] = useState<InviteResponse | null>(
    null,
  );
  const [copied, setCopied] = useState(false);

  const form = useForm<InviteFormValues>({
    defaultValues: {
      role: "member",
      ttl_hours: "24",
    },
  });

  function handleOpenChange(next: boolean) {
    onOpenChange(next);
    if (!next) {
      form.reset({ role: "member", ttl_hours: "24" });
      setCreatedInvite(null);
      setCopied(false);
    }
  }

  async function onSubmit(values: InviteFormValues) {
    const ttlHoursNumber = Number.parseInt(values.ttl_hours, 10);
    const parsed = createInviteRequestSchema.safeParse({
      role: values.role,
      ttl_hours: Number.isFinite(ttlHoursNumber) ? ttlHoursNumber : undefined,
    });
    if (!parsed.success) {
      const issue = parsed.error.issues[0];
      form.setError("root", {
        message: issue?.message ?? "Invalid invite settings",
      });
      return;
    }
    try {
      const invite = await createMutation.mutateAsync({
        orgId,
        body: parsed.data,
      });
      setCreatedInvite(invite);
      toast.success("Invite created");
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to create invite");
      }
    }
  }

  async function handleCopy() {
    if (!createdInvite) return;
    try {
      await copyToClipboard(buildOrgInviteJoinUrl(createdInvite.nonce));
      setCopied(true);
      toast.success("Invite link copied");
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast.error("Failed to copy to clipboard");
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>
            {createdInvite ? "Invite created" : "Invite a member"}
          </DialogTitle>
          <DialogDescription>
            {createdInvite
              ? "Share the link below with the recipient. They join by clicking it while signed in to NyxID."
              : "Generate a one-time invite. The recipient must be a NyxID user and will accept the invite while signed in."}
          </DialogDescription>
        </DialogHeader>

        {createdInvite ? (
          <div className="space-y-4">
            <div className="rounded-lg border border-border bg-muted/30 p-4">
              <p className="mb-2 text-xs uppercase tracking-wider text-text-tertiary">
                Invite link
              </p>
              <div className="flex items-center justify-between gap-2">
                <span className="break-all text-[12px] text-foreground">
                  {buildOrgInviteJoinUrl(createdInvite.nonce)}
                </span>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => void handleCopy()}
                  aria-label="Copy invite link"
                  className="h-8 w-8 shrink-0"
                >
                  {copied ? (
                    <Check className="h-4 w-4 text-success" />
                  ) : (
                    <Copy className="h-4 w-4" />
                  )}
                </Button>
              </div>
            </div>
            <p className="text-xs text-muted-foreground">
              Expires {new Date(createdInvite.expires_at).toLocaleString()}
            </p>
            <DialogFooter>
              <Button variant="primary" onClick={() => handleOpenChange(false)}>Done</Button>
            </DialogFooter>
          </div>
        ) : (
          <Form {...form}>
            <form
              onSubmit={(e) => void form.handleSubmit(onSubmit)(e)}
              className="space-y-4"
            >
              <FormField
                control={form.control}
                name="role"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Role</FormLabel>
                    <FormControl>
                      <Select
                        value={field.value}
                        onValueChange={(value) =>
                          field.onChange(value as OrgRole)
                        }
                      >
                        <SelectTrigger>
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          {ORG_ROLES.map((role) => (
                            <SelectItem key={role} value={role}>
                              {role.charAt(0).toUpperCase() + role.slice(1)}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="ttl_hours"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Expires in (hours)</FormLabel>
                    <FormControl>
                      <Input
                        type="number"
                        min={1}
                        max={24 * 30}
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              {form.formState.errors.root && (
                <p className="text-[12px] text-destructive">
                  {form.formState.errors.root.message}
                </p>
              )}

              <DialogFooter>
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => handleOpenChange(false)}
                  disabled={createMutation.isPending}
                >
                  Cancel
                </Button>
                <Button variant="primary" type="submit" isLoading={createMutation.isPending} disabled={!form.formState.isValid || createMutation.isPending}>
                  Create Invite
                </Button>
              </DialogFooter>
            </form>
          </Form>
        )}
      </DialogContent>
    </Dialog>
  );
}
