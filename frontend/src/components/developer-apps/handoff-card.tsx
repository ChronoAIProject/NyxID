import { useEffect } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import { useUpdateAppHandoff } from "@/hooks/use-app-connect-links";
import {
  handoffFormSchema,
  type HandoffForm,
} from "@/schemas/app-connect-links";

export function HandoffCard({
  clientId,
  blurb,
}: {
  readonly clientId: string;
  readonly blurb: string | null;
}) {
  const update = useUpdateAppHandoff(clientId);
  const form = useAppForm<HandoffForm>({
    resolver: zodResolver(handoffFormSchema),
    defaultValues: { handoff_blurb: blurb ?? "" },
  });
  const { reset } = form;
  useEffect(() => {
    reset({ handoff_blurb: blurb ?? "" });
  }, [blurb, reset]);
  async function submit(values: HandoffForm) {
    try {
      const result = await update.mutateAsync(values);
      reset({ handoff_blurb: result.handoff_blurb ?? "" });
    } catch {
      /* The mutation error is displayed below. */
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Connection page</CardTitle>
        <CardDescription>
          Your app name and this text appear above the connection checklist.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(submit)} className="space-y-4">
            {update.error && <ErrorBanner message={update.error.message} />}
            <FormField
              control={form.control}
              name="handoff_blurb"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Handoff text</FormLabel>
                  <FormControl>
                    <Input
                      {...field}
                      placeholder="Connect your accounts to continue"
                      maxLength={160}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <p className="text-[12px] text-muted-foreground">
              Up to 160 characters. The “Secured by NyxID” badge and destination
              are always shown.
            </p>
            <div className="flex justify-end gap-2">
              <Button
                type="button"
                variant="ghost"
                disabled={!form.formState.isDirty || update.isPending}
                onClick={() => form.reset()}
              >
                Cancel
              </Button>
              <Button
                type="submit"
                variant="primary"
                disabled={!form.formState.isDirty || update.isPending}
                isLoading={update.isPending}
              >
                Save
              </Button>
            </div>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}
