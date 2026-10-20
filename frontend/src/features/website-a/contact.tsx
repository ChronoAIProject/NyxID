import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { UseFormReturn } from "react-hook-form";
import { useLocation } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import {
  contactTopicFromSearch,
  createWebsiteContactSchema,
} from "@/schemas/website-contact";
import type { WebsiteContactData } from "@/schemas/website-contact";
import { useWebsite } from "./website-context";
import { websiteConfig } from "./config";
import {
  contactEmailHref,
  contactTopics,
  createContactMailto,
  openContactMailto,
} from "./contact-mailto";
import "./contact.css";

function ContactForm({ form }: { form: UseFormReturn<WebsiteContactData> }) {
  const { language } = useWebsite();
  const zh = language === "zh";
  const [mailClient, setMailClient] = useState<
    "idle" | "requested" | "unavailable"
  >("idle");
  function composeMessage(fields: WebsiteContactData) {
    try {
      openContactMailto(
        createContactMailto(websiteConfig.contactEmail, fields, language),
      );
      setMailClient("requested");
    } catch {
      setMailClient("unavailable");
      console.warn(
        "[nyxid-contact] Mail client handoff unavailable; direct email link retained.",
      );
    }
  }
  const errors = form.formState.errors;
  const error = (name: keyof WebsiteContactData) =>
    errors[name] ? (
      <p className="auth-err" id={`${name}-error`} role="alert">
        {errors[name]?.message}
      </p>
    ) : null;
  const field = (name: keyof WebsiteContactData) => ({
    "aria-invalid": !!errors[name],
    "aria-describedby": errors[name] ? `${name}-error` : undefined,
  });
  return (
    <div className="card contact-form">
      <div className="over">{zh ? "发邮件" : "Email us"}</div>
      <h2
        className="h3"
        style={{ fontSize: "var(--fs-h3)", margin: "6px 0 4px" }}
      >
        {zh ? "给我们留言" : "Drop us a note"}
      </h2>
      <p className="small" id="contact-delivery-note">
        {zh
          ? "填写后将打开邮件客户端，请在客户端确认并发送。也可直接发邮件至 "
          : "This form opens your email app. Review and send there, or email "}
        <a href={contactEmailHref(websiteConfig.contactEmail)}>
          {websiteConfig.contactEmail}
        </a>
        {zh ? "。" : " directly."}
      </p>
      <form
        id="cForm"
        noValidate
        aria-describedby="contact-delivery-note"
        onSubmit={form.handleSubmit(composeMessage)}
      >
        <div className="f2">
          <div>
            <label className="auth-label" htmlFor="cName">
              {zh ? "姓名" : "Name"}
            </label>
            <input
              id="cName"
              className="c-input"
              autoComplete="name"
              required
              {...form.register("name")}
              {...field("name")}
            />
            {error("name")}
          </div>
          <div>
            <label className="auth-label" htmlFor="cEmail">
              {zh ? "工作邮箱" : "Work email"}
            </label>
            <input
              id="cEmail"
              className="c-input"
              type="email"
              autoComplete="email"
              required
              {...form.register("email")}
              {...field("email")}
            />
            {error("email")}
          </div>
        </div>
        <div className="f2">
          <div>
            <label className="auth-label" htmlFor="cCo">
              {zh ? "公司" : "Company"}{" "}
              <span className="small">{zh ? "（选填）" : "(optional)"}</span>
            </label>
            <input
              id="cCo"
              className="c-input"
              autoComplete="organization"
              {...form.register("company")}
            />
            {error("company")}
          </div>
          <div>
            <label className="auth-label" htmlFor="cTopic">
              {zh ? "主题" : "Topic"}
            </label>
            <select
              id="cTopic"
              className="c-input"
              {...form.register("topic")}
              {...field("topic")}
            >
              <option value="">{zh ? "选择主题" : "Choose a topic"}</option>
              {contactTopics.map(([value, en, cn]) => (
                <option key={value} value={value}>
                  {zh ? cn : en}
                </option>
              ))}
            </select>
            {error("topic")}
          </div>
        </div>
        <div>
          <label className="auth-label" htmlFor="cMsg">
            {zh ? "留言" : "Message"}
          </label>
          <textarea
            id="cMsg"
            className="c-input"
            rows={5}
            required
            {...form.register("message")}
            {...field("message")}
          />
          {error("message")}
        </div>
        <button className="btn p lg" type="submit" style={{ width: "100%" }}>
          {zh ? "打开邮件客户端" : "Open email app"}
        </button>
        {mailClient !== "idle" && (
          <p className="small" role="status">
            {mailClient === "requested"
              ? zh
                ? "已请求打开邮件客户端，邮件尚未发送。请在客户端确认并发送；若未打开，可使用上方邮箱链接。"
                : "Email app requested; your message has not been sent. Review and send in your email app, or use the address above if it did not open."
              : zh
                ? "未能打开邮件客户端。请使用上方邮箱链接联系团队，填写内容仍留在此页。"
                : "Could not open your email app. Use the address above; your entries remain on this page."}
          </p>
        )}
      </form>
    </div>
  );
}

export function ContactSection({ children }: { children: ReactNode }) {
  const { language } = useWebsite();
  const location = useLocation();
  const [committedSearch, setCommittedSearch] = useState("");
  useEffect(() => {
    setCommittedSearch(location.searchStr);
  }, [location.searchStr]);
  const topic = contactTopicFromSearch(committedSearch);
  const schema = useMemo(
    () => createWebsiteContactSchema(language),
    [language],
  );
  const form = useAppForm<WebsiteContactData>({
    resolver: zodResolver(schema),
    defaultValues: { name: "", email: "", company: "", topic, message: "" },
  });
  const { setValue } = form;
  useEffect(() => {
    setValue("topic", topic, { shouldDirty: false, shouldTouch: false });
  }, [topic, setValue]);
  return (
    <div className="wrap contact">
      <ContactForm form={form} />
      <div className="contact-side">{children}</div>
    </div>
  );
}
