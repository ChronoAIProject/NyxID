import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { useWebsite } from "./website-context";
import { websiteRelease } from "./release-config";
import type { NewsletterConfig } from "./release-config";
import {
  newsletterEmailSchema,
  subscribeToNewsletter,
} from "./public-feature-adapters";
import { usePublicFeatureRequest } from "./use-public-feature-request";
export function NewsletterSignup({
  config = websiteRelease.newsletter,
}: {
  config?: NewsletterConfig | null;
}) {
  if (!config) return null;
  return <NewsletterForm config={config} />;
}
function NewsletterForm({ config }: { config: NewsletterConfig }) {
  const { language } = useWebsite(),
    zh = language === "zh";
  const form = useAppForm<{ email: string }>({
    resolver: zodResolver(newsletterEmailSchema),
    defaultValues: { email: "" },
  });
  const request = usePublicFeatureRequest();
  const [acknowledged, setAcknowledged] = useState(false);
  async function submit({ email }: { email: string }) {
    setAcknowledged(false);
    const result = await request.run((signal) =>
      subscribeToNewsletter(config, email, language, signal),
    );
    if (result) setAcknowledged(true);
  }
  return (
    <div data-website-feature="newsletter" className="website-newsletter">
      <b style={{ fontSize: 18 }}>
        {zh ? "订阅更新" : "Subscribe for updates"}
      </b>
      <p className="small">
        <a data-newsletter-privacy href={config.privacyUrl}>
          {zh ? "隐私说明" : "Privacy information"}
        </a>
        {" · "}
        <a data-newsletter-unsubscribe href={config.unsubscribeUrl}>
          {zh ? "管理订阅与退订" : "Manage or unsubscribe"}
        </a>
      </p>
      <form className="nl-form" noValidate onSubmit={form.handleSubmit(submit)}>
        <fieldset
          disabled={!request.interactive || request.pending}
          style={{
            border: 0,
            padding: 0,
            display: "flex",
            gap: 8,
            flexWrap: "wrap",
          }}
        >
          <input
            className="c-input"
            type="email"
            aria-label={zh ? "订阅邮箱" : "Newsletter email"}
            autoComplete="email"
            maxLength={254}
            required
            {...form.register("email")}
          />
          <button
            className="btn"
            type="submit"
            disabled={!request.interactive || request.pending}
          >
            {zh ? "订阅" : "Subscribe"}
          </button>
        </fieldset>
      </form>
      {form.formState.errors.email && (
        <p role="alert" className="small">
          {zh ? "请填写有效邮箱。" : "Enter a valid email."}
        </p>
      )}
      {acknowledged && (
        <p role="status" className="small">
          {zh
            ? "请求已接收，请检查确认邮件；订阅尚未确认。"
            : "Request received. Check your confirmation email; your subscription is not confirmed yet."}
        </p>
      )}
      {request.failed && (
        <p role="alert" className="small">
          {zh
            ? "未能完成请求，请重试或通过 Discord 联系团队。"
            : "Request could not be completed. Retry or contact the team on Discord."}
        </p>
      )}
    </div>
  );
}
