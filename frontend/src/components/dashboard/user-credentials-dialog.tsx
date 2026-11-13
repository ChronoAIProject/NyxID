import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useState } from "react";
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
  useAppForm,
  Form,
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@/components/ui/form";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { OAuthCallbackGuidance } from "@/components/shared/twitter-oauth-guidance";
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
  useUiTranslation();
  const { data: credentials, isLoading } = useMyProviderCredentials(
    provider.id,
  );
  const setMutation = useSetProviderCredentials();
  const deleteMutation = useDeleteProviderCredentials();
  const [confirmDelete, setConfirmDelete] = useState(false);

  const hasExisting = credentials?.has_credentials === true;

  const form = useAppForm<UserCredentialsFormData>({
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
        client_id: data.client_id.trim(),
        client_secret:
          data.client_secret && data.client_secret.trim().length > 0
            ? data.client_secret.trim()
            : undefined,
        label:
          data.label && data.label.trim().length > 0
            ? data.label.trim()
            : undefined,
      });
      toast.success(
        translateUi(hasExisting ? "OAuth credentials updated" : "OAuth credentials saved"),
      );
      onClose();
    } catch (error) {
      if (error instanceof ApiError) {
        form.setError("root", { message: error.message });
      } else {
        toast.error(translateErrorMessage("Failed to save credentials"));
      }
    }
  }

  async function handleDelete() {
    try {
      await deleteMutation.mutateAsync(provider.id);
      toast.success(translateUi("OAuth credentials removed"));
      onClose();
    } catch (error) {
      if (error instanceof ApiError) {
        toast.error(translateErrorMessage(error.message));
      } else {
        toast.error(translateErrorMessage("Failed to remove credentials"));
      }
    }
  }

  return (
    <Dialog open onOpenChange={onClose}>
      <DialogContent className="md:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {hasExisting ? translateUi("Manage") : translateUi("Setup")}{translateUi(" OAuth App for ")}{provider.name}
          </DialogTitle>
          <DialogDescription>{translateUi("Enter your own OAuth app credentials to connect with ")}{provider.name}
            {translateUi(". Public or PKCE-only clients can leave Client Secret blank. ")}</DialogDescription>
        </DialogHeader>

        {isLoading ? (
          <div className="py-8 text-center text-14 text-muted-foreground">{translateUi("Loading...")}</div>
        ) : (
          <>
            <ExistingCredentialsInfo credentials={credentials} />

            {/* Authorization-code OAuth providers redirect through
                NyxID's callback; show the URL the user must whitelist.
                Device-code providers don't use a redirect URI. */}
            {provider.provider_type === "oauth2" && (
              <OAuthCallbackGuidance slug={provider.slug} />
            )}

            {provider.documentation_url && (
              <a
                href={provider.documentation_url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex items-center gap-1.5 type-row-title text-primary-text hover:underline"
              >{translateUi("How to create an OAuth app")}<ExternalLink className="h-3 w-3" />
              </a>
            )}

            <Form {...form}>
              <form
                onSubmit={form.handleSubmit(onSubmit)}
                className="space-y-4"
              >
                {form.formState.errors.root && (
                  <div className="rounded-lg bg-destructive/10 p-3 text-14 text-destructive">
                    {form.formState.errors.root.message}
                  </div>
                )}

                <FormField
                  control={form.control}
                  name="client_id"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Client ID")}</FormLabel>
                      <FormControl>
                        <Input
                          placeholder={translateUi("Your OAuth app Client ID")}
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
                      <FormLabel>{translateUi("Client Secret (optional)")}</FormLabel>
                      <FormControl>
                        <Input
                          type="password"
                          placeholder={translateUi("Your OAuth app Client Secret")}
                          autoComplete="off"
                          {...field}
                        />
                      </FormControl>
                      <p className="text-13 text-muted-foreground">{translateUi("Leave blank for public clients that do not use a client secret.")}</p>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <FormField
                  control={form.control}
                  name="label"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>{translateUi("Label (optional)")}</FormLabel>
                      <FormControl>
                        <Input
                          placeholder={translateUi("e.g., My Dev App")}
                          maxLength={200}
                          {...field}
                        />
                      </FormControl>
                      <FormMessage />
                    </FormItem>
                  )}
                />

                <DialogFooter>
                  {hasExisting && !confirmDelete && (
                    <Button
                      type="button"
                      variant="destructive"
                      onClick={() => setConfirmDelete(true)}
                      className="mr-auto"
                    >
                      <ButtonIcon variant="destructive"><Trash2 className="h-3 w-3 text-destructive" /></ButtonIcon>{translateUi("Remove")}</Button>
                  )}

                  {confirmDelete && (
                    <div className="mr-auto flex items-center gap-2">
                      <Button
                        type="button"
                        variant="destructive"
                        onClick={() => void handleDelete()}
                        isLoading={deleteMutation.isPending}
                      >{translateUi("Confirm Remove")}</Button>
                      <Button
                        type="button"
                        variant="ghost"
                        onClick={() => setConfirmDelete(false)}
                      >{translateUi("Cancel")}</Button>
                    </div>
                  )}

                  <Button
                    type="button"
                    variant="outline"
                    onClick={onClose}
                    disabled={setMutation.isPending}
                  >{translateUi("Cancel")}</Button>
                  <Button variant="primary" type="submit" isLoading={setMutation.isPending} disabled={!form.formState.isValid || setMutation.isPending}>
                    {hasExisting ? translateUi("Update") : translateUi("Save")}
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
  useUiTranslation();
  if (!credentials?.has_credentials) return null;

  return (
    <div className="flex items-center gap-2 rounded-lg bg-muted p-3 text-14">
      <Badge variant="success">{translateUi("Configured")}</Badge>
      <div className="flex flex-col gap-0.5 text-13 text-muted-foreground">
        {credentials.label && <span>{credentials.label}</span>}
        {credentials.updated_at && (
          <span>{translateUi("Updated ")}{formatDate(credentials.updated_at)}</span>
        )}
      </div>
    </div>
  );
}
