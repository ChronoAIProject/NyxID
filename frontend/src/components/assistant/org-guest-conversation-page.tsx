import { useNavigate } from "@tanstack/react-router";
import { Button } from "@/components/ui/button";
import { useOrgGuestHistory } from "@/hooks/use-org-guest-conversations";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { channelPlatformName } from "@/lib/assistant/nyxbot-labels";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { AssistantEngineSidebar } from "./assistant-engine-sidebar";
import { AssistantShell } from "./assistant-shell";

export function OrgGuestConversationPage({ id }: { readonly id: string }) {
  const { t } = useUiTranslation();
  const navigate = useNavigate();
  const query = useOrgGuestHistory(id);
  const pages = query.isError ? undefined : query.data?.pages;
  const source = pages?.[0]?.conversation.channel;
  const messages = pages?.slice().reverse().flatMap((page) => page.messages) ?? [];
  function go(c?: string) {
    void navigate({ to: "/assistant", search: c ? { c } : {} });
  }
  return (
    <AssistantShell title={t("Guest conversation")} sidebar={
      <AssistantEngineSidebar engine="nyxagent" conversations={[]} activeConversationId={id}
        guestAgentId={pages?.[0]?.conversation.agent_id}
        onNewChat={() => go()} onSelect={go} onDelete={() => undefined} />
    }>
      <div className="assistant-scrollbar h-full overflow-y-auto">
        <div className="mx-auto max-w-3xl space-y-5 px-4 py-6">
          <header className="border-b border-hairline pb-4">
            <h1 className="type-card-title">{t("Read-only guest conversation")}</h1>
            {source ? (
              <p className="mt-1 type-secondary text-text-tertiary">
                {[source.parent_title ?? source.chat_title, channelPlatformName(source.platform), source.bot_label].filter(Boolean).join(" · ")}
              </p>
            ) : null}
          </header>
          {query.isError ? <p role="alert">{t("Could not load the guest conversation.")}</p> : null}
          {query.isPending ? <p role="status">{t("Loading conversation...")}</p> : null}
          {pages && query.hasNextPage ? (
            <Button variant="outline" disabled={query.isFetchingNextPage} onClick={() => void query.fetchNextPage()}>
              {t("Load earlier messages")}
            </Button>
          ) : null}
          <ol className="space-y-4">
            {messages.map((message) => (
              <li key={message.seq} className="rounded-lg border border-hairline px-4 py-3">
                <div className="mb-2 flex flex-wrap items-baseline justify-between gap-2 type-secondary text-text-tertiary">
                  <span>{message.role === "user" ? t("Guest") : t("Assistant")}</span>
                  <time dateTime={message.created_at}>{new Date(message.created_at).toLocaleString(getFormattingLocale())}</time>
                </div>
                {message.status === "completed" && message.text ? (
                  <p className="whitespace-pre-wrap break-words">{message.text}</p>
                ) : null}
                <div className="space-y-1 type-secondary text-text-tertiary">
                  {message.error_code ? <p>{t("Turn outcome: {{code}}", { code: message.error_code })}</p> : null}
                  {message.delivery_status ? <p>{t("Delivery outcome: {{code}}", { code: message.delivery_status })}</p> : null}
                  {message.tool_failures?.map(({ kind, reason, status }, index) => (
                    <p key={index}>{t("Tool outcome: {{code}}", { code: [reason ?? kind, status].filter(Boolean).join(" · ") })}</p>
                  ))}
                </div>
              </li>
            ))}
          </ol>
        </div>
      </div>
    </AssistantShell>
  );
}
