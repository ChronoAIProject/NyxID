import { useState } from "react";
import { useForm } from "react-hook-form";
import { zodResolver } from "@hookform/resolvers/zod";
import type { ProviderConfig, UserProviderCredentials } from "@/types/api";
import {
  useSetProviderCredentials,
  useDeleteProviderCredentials,
  useMyProviderCredentials,
} from "@/hooks/use-providers";
import {
  userCredentialsSchema,
  type UserCredentialsFormData,
} from "@/schemas/providers";
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
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { ExternalLink, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { ApiError } from "@/lib/api-client";
import { formatDate } from "@/lib/utils";

interface UserCredentialsDialogProps {
  readonly provider: ProviderConfig;
  readonly onClose: () => void;
}

export function UserCredentialsDialog({
  provider,
  onClose,
}: UserCredentialsDialogProps) {
  const { data: credentials, isLoading } = useMyProviderCredentials(
    provider.id,
  );
  const setMutation = useSetProviderCredentials();
  const deleteMutation = useDeleteProviderCredentials();
  const [confirmDelete, setConfirmDelete] = useState(false);

  const hasExisting = credentials?.has_credentials === true;

  const form = useForm<UserCredentialsFormData>({
    resolver: zodResolver(userCredentialsSchema),
    defaultValues: {
      client_id: "",
      client_secret: "",
      label: "",
    },
  });

  async function onSubmit(data: UserCredentialsFormData) {
    try {
      await setMutation.mutateAsync({
        providerId: provider.id,
        client_id: data.client_id,
        client_secret:
          data.client_secret && data.client_secret.trim().length > 0
            ? data.client_secret
            : undefined,
        label:
          data.label && data.label.trim().length > 0
            ? data.label.trim()
            : undefined,
      });
      toast.success(
        hasExisting ? "OAuth credentials updated" : "OAuth credentials saved",
      );
      onClose();
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error("Failed to save credentials");
      }
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(provider.id);
      toast.success("OAuth credentials removed");
      onClose();
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(error.message);
      } else {
        toast.error("Failed to remove credentials");
      }
    }
  }

  return (
    <Dialog open onOpenChange={onClose}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {hasExisting ? "Manage" : "Setup"} OAuth App for {provider.name}
          </DialogTitle>
          <DialogDescription>
            Enter your own OAuth app credentials to connect with {provider.name}
            . Public or PKCE-only clients can leave Client Secret blank.
          </DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="py-8 text-center text-sm text-muted-foreground">
            Loading...
          </div>
        ) : (
          <>
            <ExistingCredentialsInfo credentials={credentials} />

            {provider.documentation_url && (
              <a
                href={provider.documentation_url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 text-sm text-primary hover:underline"
              >
                How to create an OAuth app
                <ExternalLink className="h-3 w-3" />
              </a>
            )}

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
                  name="client_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Client ID</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="Your OAuth app Client ID"
                          autoComplete="off"
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="client_secret"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Client Secret (optional)</FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder="Your OAuth app Client Secret"
                          autoComplete="off"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-xs text-muted-foreground">
                        Leave blank for public clients that do not use a client
                        secret.
                      </p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="label"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Label (optional)</FormLabel>
                      <FormControl>
                        <Input
                          placeholder="e.g., My Dev App"
                          maxLength={200}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <DialogFooter className="flex-col gap-2 sm:flex-row">
                  {hasExisting && !confirmDelete && (
                    <Button
                      type="button"
                      variant="destructive"
                      size="sm"
                      onClick={() => setConfirmDelete(true)}
                      className="mr-auto"
                    >
                      <Trash2 className="mr-1 h-3 w-3" />
                      Remove
                    </Button>
                  )}

                  {confirmDelete && (
                    <div className="mr-auto flex items-center gap-2">
                      <Button
                        type="button"
                        variant="destructive"
                        size="sm"
                        onClick={() => void handleDelete()}
                        isLoading={deleteMutation.isPending}
                      >
                        Confirm Remove
                      </Button>
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        onClick={() => setConfirmDelete(false)}
                      >
                        Cancel
                      </Button>
                    </div>
                  )}

                  <Button
                    type="button"
                    variant="outline"
                    onClick={onClose}
                    disabled={setMutation.isPending}
                  >
                    Cancel
                  </Button>
                  <Button type="submit" isLoading={setMutation.isPending}>
                    {hasExisting ? "Update" : "Save"}
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

function ExistingCredentialsInfo({
  credentials,
}: {
  readonly credentials: UserProviderCredentials | undefined;
}) {
  if (!credentials?.has_credentials) return null;

  return (
    <div className="flex items-center gap-2 rounded-md bg-muted p-3 text-sm">
      <Badge variant="success">Configured</Badge>
      <div className="flex flex-col gap-0.5 text-xs text-muted-foreground">
        {credentials.label && <span>{credentials.label}</span>}
        <span>Updated {formatDate(credentials.updated_at)}</span>
      </div>
    </div>
  );
}
