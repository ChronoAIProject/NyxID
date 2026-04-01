import { useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useInView } from "../hooks/use-in-view";

const MC_ACTION = "https://fun.us9.list-manage.com/subscribe/post";
const MC_U = "eecb4f508c0388d7720a99c82";
const MC_ID = "0ad1109319";
const MC_FID = "00d454e1f0";
const MC_HONEYPOT = "b_eecb4f508c0388d7720a99c82_0ad1109319";

type FormData = {
  FNAME: string;
  LNAME: string;
  EMAIL: string;
  COMPANY: string;
};

export function WaitlistForm() {
  const { ref, inView } = useInView();
  const { t } = useTranslation();
  const formRef = useRef<HTMLFormElement>(null);
  const [form, setForm] = useState<FormData>({
    FNAME: "",
    LNAME: "",
    EMAIL: "",
    COMPANY: "",
  });
  const [submitted, setSubmitted] = useState(false);
  const [error, setError] = useState("");

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");

    if (!form.FNAME.trim() || !form.EMAIL.trim()) {
      setError(t("waitlist.errorRequired"));
      return;
    }

    const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
    if (!emailRegex.test(form.EMAIL)) {
      setError(t("waitlist.errorEmail"));
      return;
    }

    const params = new URLSearchParams({
      u: MC_U,
      id: MC_ID,
      f_id: MC_FID,
      FNAME: form.FNAME,
      LNAME: form.LNAME,
      EMAIL: form.EMAIL,
      COMPANY: form.COMPANY,
      tags: "11843820",
      [MC_HONEYPOT]: "",
    });

    const url =
      MC_ACTION.replace("/post", "/post-json") +
      "?" +
      params.toString() +
      "&c=__mc_cb";

    (window as unknown as Record<string, unknown>).__mc_cb = () => {
      delete (window as unknown as Record<string, unknown>).__mc_cb;
    };
    const script = document.createElement("script");
    script.src = url;
    document.body.appendChild(script);
    script.onload = () => script.remove();
    script.onerror = () => script.remove();

    setSubmitted(true);
  };

  const update =
    (field: keyof FormData) => (e: React.ChangeEvent<HTMLInputElement>) =>
      setForm((prev) => ({ ...prev, [field]: e.target.value }));

  if (submitted) {
    return (
      <section id="waitlist" className="px-6 py-24">
        <div className="mx-auto max-w-lg rounded-2xl border border-primary/30 bg-landing-surface p-12 text-center">
          <div className="mx-auto mb-4 flex h-16 w-16 items-center justify-center rounded-full bg-primary/10">
            <svg
              width="32"
              height="32"
              viewBox="0 0 24 24"
              fill="none"
              stroke="#8B5CF6"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </div>
          <h3 className="mb-2 font-serif text-2xl text-white">
            {t("waitlist.successTitle")}
          </h3>
          <p className="text-gray-300">{t("waitlist.successMessage")}</p>
        </div>
      </section>
    );
  }

  return (
    <section id="waitlist" className="px-6 py-24" ref={ref}>
      <div
        className={`mx-auto max-w-lg ${
          inView ? "animate-fade-up" : "opacity-0"
        }`}
      >
        <h2 className="mb-4 text-center font-serif text-3xl text-white md:text-4xl">
          {t("waitlist.heading")}
        </h2>
        <p className="mx-auto mb-2 max-w-md text-center text-gray-300">
          {t("waitlist.subheading")}
        </p>
        <p className="mb-10 text-center font-mono text-sm text-primary">
          {t("waitlist.socialProof")}
        </p>

        <form
          ref={formRef}
          onSubmit={handleSubmit}
          className="rounded-2xl border border-landing-border-subtle bg-landing-surface p-8"
        >
          <div className="mb-4">
            <label
              htmlFor="mce-FNAME"
              className="mb-1.5 block font-mono text-xs text-gray-400"
            >
              {t("waitlist.labelName")}{" "}
              <span className="text-primary">*</span>
            </label>
            <input
              id="mce-FNAME"
              name="FNAME"
              type="text"
              required
              value={form.FNAME}
              onChange={update("FNAME")}
              className="w-full rounded-lg border border-landing-border-subtle bg-landing-bg px-4 py-3 text-white placeholder-gray-600 outline-none transition-colors focus:border-primary"
              placeholder={t("waitlist.placeholderName")}
            />
          </div>

          <div className="mb-4">
            <label
              htmlFor="mce-LNAME"
              className="mb-1.5 block font-mono text-xs text-gray-400"
            >
              {t("waitlist.labelLastName")}
            </label>
            <input
              id="mce-LNAME"
              name="LNAME"
              type="text"
              value={form.LNAME}
              onChange={update("LNAME")}
              className="w-full rounded-lg border border-landing-border-subtle bg-landing-bg px-4 py-3 text-white placeholder-gray-600 outline-none transition-colors focus:border-primary"
              placeholder={t("waitlist.placeholderLastName")}
            />
          </div>

          <div className="mb-4">
            <label
              htmlFor="mce-EMAIL"
              className="mb-1.5 block font-mono text-xs text-gray-400"
            >
              {t("waitlist.labelEmail")}{" "}
              <span className="text-primary">*</span>
            </label>
            <input
              id="mce-EMAIL"
              name="EMAIL"
              type="email"
              required
              value={form.EMAIL}
              onChange={update("EMAIL")}
              className="w-full rounded-lg border border-landing-border-subtle bg-landing-bg px-4 py-3 text-white placeholder-gray-600 outline-none transition-colors focus:border-primary"
              placeholder={t("waitlist.placeholderEmail")}
            />
          </div>

          <div className="mb-6">
            <label
              htmlFor="mce-COMPANY"
              className="mb-1.5 block font-mono text-xs text-gray-400"
            >
              {t("waitlist.labelCompany")}
            </label>
            <input
              id="mce-COMPANY"
              name="COMPANY"
              type="text"
              value={form.COMPANY}
              onChange={update("COMPANY")}
              className="w-full rounded-lg border border-landing-border-subtle bg-landing-bg px-4 py-3 text-white placeholder-gray-600 outline-none transition-colors focus:border-primary"
              placeholder={t("waitlist.placeholderCompany")}
            />
          </div>

          {error && <p className="mb-4 text-sm text-destructive">{error}</p>}

          <button
            type="submit"
            className="w-full rounded-lg bg-primary py-3.5 font-semibold text-white transition-colors hover:bg-void-400"
          >
            {t("waitlist.submit")}
          </button>

          <p className="mt-4 text-center text-xs text-gray-500">
            {t("waitlist.disclaimer")}
          </p>
        </form>
      </div>
    </section>
  );
}
