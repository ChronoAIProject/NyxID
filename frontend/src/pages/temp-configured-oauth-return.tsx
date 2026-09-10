import { ConfiguredOAuthConnect } from "@/components/shared/configured-oauth-connect";

const PAGE_OPTIONS = [
  { value: "local-poc", label: "Local POC" },
  { value: "onboarding", label: "Onboarding" },
  { value: "dashboard", label: "Dashboard" },
  { value: "default", label: "Safe default" },
] as const;

export function TempConfiguredOAuthReturn({
  userId,
}: {
  readonly userId: string;
}) {
  return (
    <section
      aria-labelledby="configured-return-heading"
      className="grid gap-6 border-t border-border py-8 sm:grid-cols-[180px_1fr]"
    >
      <h2 id="configured-return-heading" className="text-lg font-medium">
        Try a configured return
      </h2>
      <ConfiguredOAuthConnect
        userId={userId}
        serviceSlug="api-google"
        serviceName="Google"
        returnPage="local-poc"
        pageOptions={PAGE_OPTIONS}
        showDestination
      />
    </section>
  );
}
