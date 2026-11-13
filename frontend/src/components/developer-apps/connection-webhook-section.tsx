import { translateErrorMessage } from "@/lib/error-messages";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useEffect, useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { RotateCw, Webhook } from "lucide-react";
import { toast } from "sonner";
import {
  useConfigureConnectionWebhook,
  useDisableConnectionWebhook,
  useRotateConnectionWebhookSecret,
} from "@/hooks/use-connection-webhooks";
import {
  connectionWebhookFormSchema,
  type ConnectionWebhookForm,
} from "@/schemas/connection-webhooks";
import { ApiError } from "@/lib/api-client";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
  useAppForm,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";
import {
  OneTimeSecretDialog,
  type OneTimeSecretValue,
} from "@/components/shared/one-time-secret-dialog";

interface ConnectionWebhookSectionProps {
  readonly clientId: string;
  readonly webhookUrl: string | null;
  readonly enabled: boolean;
}

export function ConnectionWebhookSection({
  clientId,
  webhookUrl,
  enabled,
}: ConnectionWebhookSectionProps) {
  const { t } = useUiTranslation();
  const configureMutation = useConfigureConnectionWebhook();
  const rotateMutation = useRotateConnectionWebhookSecret();
  const disableMutation = useDisableConnectionWebhook();
  const [secretValues, setSecretValues] = useState<
    readonly OneTimeSecretValue[]
  >([]);
  const [secretOpen, setSecretOpen] = useState(false);
  const [rotateOpen, setRotateOpen] = useState(false);
  const [disableOpen, setDisableOpen] = useState(false);
  const form = useAppForm<ConnectionWebhookForm>({
    resolver: zodResolver(connectionWebhookFormSchema),
    mode: "onChange",
    defaultValues: { url: webhookUrl ?? "" },
  });

  useEffect(() => {
    form.reset({ url: webhookUrl ?? "" });
  }, [form, webhookUrl]);

  function revealSecret(signingSecret: string) {
    setSecretValues([{ label: "Signing Secret", value: signingSecret }]);
    setSecretOpen(true);
  }

  async function configure(values: ConnectionWebhookForm) {
    try {
      const response = await configureMutation.mutateAsync({
        clientId,
        url: values.url,
      });
      form.reset({ url: response.connection_webhook_url });
      revealSecret(response.signing_secret);
      toast.success(translateUi("Connection webhook configured"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError
          ? error.message
          : "Failed to configure connection webhook"),
      );
    }
  }

  async function rotate() {
    try {
      const response = await rotateMutation.mutateAsync(clientId);
      setRotateOpen(false);
      revealSecret(response.signing_secret);
      toast.success(translateUi("Connection webhook secret rotated"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError
          ? error.message
          : "Failed to rotate connection webhook secret"),
      );
    }
  }

  async function disable() {
    try {
      await disableMutation.mutateAsync(clientId);
      setDisableOpen(false);
      form.reset({ url: "" });
      toast.success(translateUi("Connection webhook disabled"));
    } catch (error) {
      toast.error(
        translateErrorMessage(error instanceof ApiError
          ? error.message
          : "Failed to disable connection webhook"),
      );
    }
  }

  return (
    <>
      <Card>
        <CardHeader>
          <div className="flex items-start justify-between gap-4">
            <div className="space-y-1.5">
              <CardTitle className="flex items-center gap-2">
                <Webhook className="h-4 w-4 text-muted-foreground" />{t("Connection Webhook")}</CardTitle>
              <CardDescription>{t("Receive signed connection lifecycle events at your server.")}</CardDescription>
            </div>
            <Badge variant={enabled ? "success" : "secondary"}>
              {enabled ? t("Enabled") : t("Not Configured")}
            </Badge>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <Form {...form}>
            <form className="space-y-4" onSubmit={form.handleSubmit(configure)}>
              <FormField
                control={form.control}
                name="url"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>{t("Webhook URL")}</FormLabel>
                    <FormControl>
                      <Input
                        {...field}
                        type="url"
                        placeholder="https://events.example.com/nyxid"
                        autoComplete="url"
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <div className="flex flex-wrap justify-end gap-2">
                {enabled && (
                  <>
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => setRotateOpen(true)}
                    >
                      <ButtonIcon>
                        <RotateCw className="h-3 w-3" />
                      </ButtonIcon>{t("Rotate Secret")}</Button>
                    <Button
                      type="button"
                      variant="destructive"
                      onClick={() => setDisableOpen(true)}
                    >{t("Disable Webhook")}</Button>
                  </>
                )}
                <Button
                  type="submit"
                  variant="primary"
                  disabled={
                    !form.formState.isDirty ||
                    !form.formState.isValid ||
                    configureMutation.isPending
                  }
                  isLoading={configureMutation.isPending}
                >
                  {enabled ? t("Save Changes") : t("Configure Webhook")}
                </Button>
              </div>
            </form>
          </Form>
        </CardContent>
      </Card>

      <OneTimeSecretDialog
        open={secretOpen}
        onOpenChange={setSecretOpen}
        title={t("Save Connection Webhook Secret")}
        description={t("This signing secret is shown only once. Copy and store it securely now.")}
        values={secretValues}
      />

      <Dialog open={rotateOpen} onOpenChange={setRotateOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Rotate webhook secret?")}</DialogTitle>
            <DialogDescription>{t("The current signing secret will stop working immediately. Update every webhook receiver with the new value.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setRotateOpen(false)}>{t("Cancel")}</Button>
            <Button
              variant="primary"
              isLoading={rotateMutation.isPending}
              onClick={() => void rotate()}
            >{t("Confirm Rotation")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={disableOpen} onOpenChange={setDisableOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{t("Disable connection webhook?")}</DialogTitle>
            <DialogDescription>{t("Connection lifecycle events will no longer be delivered to this endpoint.")}</DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setDisableOpen(false)}>{t("Cancel")}</Button>
            <Button
              variant="destructive"
              isLoading={disableMutation.isPending}
              onClick={() => void disable()}
            >{t("Confirm Disable")}</Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
