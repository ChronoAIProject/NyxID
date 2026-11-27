import { useEffect, useState } from "react";
import { CopyableField } from "@/components/shared/copyable-field";
import { Button } from "@/components/ui/button";
import { GroupedRow } from "@/components/ui/grouped-list";
import {
  useNotificationSettings,
  useTelegramLink,
} from "@/hooks/use-approvals";
import { translateUi } from "@/i18n/use-ui-translation";

/** Mounted only while the account has no linked Telegram chat. */
export function ApprovalTelegramSetup({ disabled }: { disabled?: boolean }) {
  const link = useTelegramLink();
  const code = link.data;
  const [expired, setExpired] = useState(false);
  useNotificationSettings({
    enabled: !!code && !expired,
    refetchInterval: 4000,
  });

  useEffect(() => {
    if (!code) return;
    const timer = window.setTimeout(
      () => setExpired(true),
      code.expires_in_secs * 1000,
    );
    return () => window.clearTimeout(timer);
  }, [code]);

  function requestCode() {
    setExpired(false);
    link.mutate();
  }

  return (
    <>
      <GroupedRow
        title={translateUi("Telegram")}
        description={
          code && !expired
            ? translateUi("Open the chat with @{{bot}} and tap Start.", {
                bot: code.bot_username,
              })
            : translateUi("Telegram is not linked to your account.")
        }
        accessory={
          code && !expired ? (
            <Button asChild variant="outline" size="sm">
              <a
                href={`https://t.me/${encodeURIComponent(code.bot_username)}?start=${encodeURIComponent(code.link_code)}`}
                target="_blank"
                rel="noopener noreferrer"
              >
                {translateUi("Open Telegram")}
              </a>
            </Button>
          ) : (
            <Button
              variant="outline"
              size="sm"
              disabled={disabled || link.isPending}
              isLoading={link.isPending}
              onClick={requestCode}
            >
              {expired ? translateUi("Get a new code") : translateUi("Connect")}
            </Button>
          )
        }
      />
      {code && !expired && (
        <GroupedRow>
          <div className="w-full space-y-2">
            <CopyableField
              size="sm"
              label={translateUi("Or send this command to @{{bot}}", {
                bot: code.bot_username,
              })}
              value={`/start ${code.link_code}`}
            />
            <p role="status" className="type-secondary text-text-tertiary">
              {translateUi(
                "Waiting for Telegram. The code is valid for {{minutes}} min.",
                {
                  minutes: Math.ceil(code.expires_in_secs / 60),
                },
              )}
            </p>
          </div>
        </GroupedRow>
      )}
      {(expired || link.isError) && (
        <GroupedRow>
          <p role="alert" className="type-secondary text-foreground">
            {expired
              ? translateUi("This Telegram code has expired.")
              : translateUi("Failed to generate link code")}
          </p>
        </GroupedRow>
      )}
    </>
  );
}
