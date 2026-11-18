import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useId, useState, type RefObject } from "react";
import { Check, Link2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { groupServiceConnections } from "@/lib/service-groups";
import {
  composerConnectionOwnerLabel,
  composerConnectionUnavailableReason,
  formatComposerConnectionReason,
} from "@/lib/assistant/composer-connection";
import type { KeyInfo } from "@/types/keys";

export function ComposerConnectionPicker({
  open,
  onOpenChange,
  connections,
  value,
  onChange,
  loading,
  error,
  onRetry,
  disabled,
  now,
  onCloseAutoFocus,
  checkingId,
  onCancelPending,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly connections: readonly KeyInfo[];
  readonly value: string;
  readonly onChange: (id: string) => "accepted" | "held" | "refused" | void;
  readonly loading: boolean;
  readonly error: boolean;
  readonly onRetry: () => void;
  readonly disabled: boolean;
  readonly now?: number;
  readonly checkingId?: string;
  readonly onCancelPending?: () => void;
  readonly onCloseAutoFocus?: () => void;
}) {
  useUiTranslation();
  const reasonId = useId();
  const [search, setSearch] = useState("");
  const [activationNow, setActivationNow] = useState(0);
  const [wasOpen, setWasOpen] = useState(open);
  // Parent-driven closes (navigation or a held choice settling) reset search too.
  if (wasOpen !== open) {
    setWasOpen(open);
    if (!open) setSearch("");
  }
  const query = search.trim().toLocaleLowerCase();
  const groups = groupServiceConnections(connections)
    .map((group) => ({
      ...group,
      connections: group.connections.filter((key) =>
        `${group.name} ${key.label} ${key.slug} ${composerConnectionOwnerLabel(key)}`
          .toLocaleLowerCase()
          .includes(query),
      ),
    }))
    .filter((group) => group.connections.length);

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        setSearch("");
        onOpenChange(next);
      }}
    >
      <DialogContent
        className="max-w-md"
        onCloseAutoFocus={
          onCloseAutoFocus
            ? (event) => {
                event.preventDefault();
                onCloseAutoFocus();
              }
            : undefined
        }
      >
        <DialogHeader>
          <DialogTitle>
            {translateUi("Connection for this message")}
          </DialogTitle>
          <DialogDescription>
            {translateUi(
              "Choose the account to use for this message. Manage your saved connection order in Agent Details.",
            )}
          </DialogDescription>
        </DialogHeader>
        <Input
          aria-label={translateUi("Search connections")}
          placeholder={translateUi("Search services or accounts…")}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
        />
        <div className="assistant-scrollbar max-h-[50vh] space-y-4 overflow-y-auto">
          {loading && !connections.length && (
            <p role="status" className="text-14 text-muted-foreground">
              {translateUi("Loading connections…")}
            </p>
          )}
          {error ? (
            <div role="alert" className="space-y-2 text-14">
              <p>{translateUi("Could not load connections.")}</p>
              <Button type="button" onClick={onRetry}>
                {translateUi("Retry")}
              </Button>
            </div>
          ) : (
            groups.map((group) => (
              <section
                key={group.id}
                aria-label={group.name}
                className="space-y-1"
              >
                <h3 className="type-body-strong">{group.name}</h3>
                {group.connections.map((key) => {
                  const reason = composerConnectionUnavailableReason(key, Math.max(now ?? Date.now(), activationNow));
                  return (
                    <button
                      key={key.id}
                      type="button"
                      disabled={disabled}
                      aria-disabled={Boolean(reason)}
                      aria-busy={checkingId === key.id || undefined}
                      aria-labelledby={`${reasonId}-${key.id}-label ${reasonId}-${key.id}-owner`}
                      aria-describedby={
                        reason || checkingId === key.id ? `${reasonId}-${key.id}` : undefined
                      }
                      aria-pressed={value === key.id}
                      className="flex w-full items-center gap-2 rounded-lg px-2 py-2 text-left text-14 enabled:hover:bg-overlay aria-disabled:hover:bg-transparent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring aria-disabled:cursor-not-allowed aria-disabled:text-muted-foreground disabled:cursor-not-allowed disabled:text-muted-foreground"
                      onClick={() => {
                        if (disabled) return;
                        onCancelPending?.();
                        const currentNow = Math.max(now ?? 0, Date.now());
                        setActivationNow(currentNow);
                        if (composerConnectionUnavailableReason(key, currentNow)) return;
                        const result = onChange(key.id);
                        if (result === undefined || result === "accepted") {
                          onOpenChange(false);
                          setSearch("");
                        }
                      }}
                    >
                      <span className="min-w-0 flex-1 break-words">
                        <span
                          id={`${reasonId}-${key.id}-label`}
                          className="block font-medium"
                        >
                          {key.label}
                        </span>
                        <span
                          id={`${reasonId}-${key.id}-owner`}
                          className="text-13 text-muted-foreground"
                        >
                          {composerConnectionOwnerLabel(key)} · {key.slug}
                        </span>
                        {(reason || checkingId === key.id) && (
                          <span
                            id={`${reasonId}-${key.id}`}
                            className={checkingId === key.id
                              ? "block type-secondary text-muted-foreground"
                              : "block type-secondary text-destructive"}
                          >
                            {checkingId === key.id ? translateUi("Checking availability…") : reason && formatComposerConnectionReason(reason)}
                          </span>
                        )}
                      </span>
                      {value === key.id && (
                        <Check aria-hidden className="size-4 shrink-0" />
                      )}
                    </button>
                  );
                })}
              </section>
            ))
          )}
          {!loading && !error && !groups.length && (
            <p className="text-14 text-muted-foreground">
              {query
                ? translateUi("No matching connections.")
                : translateUi("No connections are available.")}
            </p>
          )}
        </div>
        <p className="text-13 text-muted-foreground">
          {translateUi(
            "Organization connections still require approval in Auto and Ask every time.",
          )}
        </p>
      </DialogContent>
    </Dialog>
  );
}

export function ComposerConnectionChip({
  connection,
  unavailableReason,
  disabled,
  onRemove,
  onChange,
  changeButtonRef,
  checking = false,
  announceChecking = false,
}: {
  readonly connection?: KeyInfo;
  readonly changeButtonRef?: RefObject<HTMLButtonElement | null>;
  readonly unavailableReason: string | null;
  readonly disabled: boolean;
  readonly onRemove: () => void;
  readonly onChange: () => void;
  readonly checking?: boolean;
  readonly announceChecking?: boolean;
}) {
  useUiTranslation();
  const [settledReason, setSettledReason] = useState(unavailableReason);
  if (!checking && settledReason !== unavailableReason) setSettledReason(unavailableReason);
  // Background refreshes leave the live region's settled text intact. Only a
  // refused send announces checking; the background visual is hidden from AT.
  const statusReason = checking ? settledReason : unavailableReason;
  const quietChecking = checking && !announceChecking;
  return (
    <div
      role="group"
      aria-label={translateUi("Connection for this message")}
      className="mb-2 ml-[30px] flex min-w-0 items-center gap-2 rounded-lg border border-border px-2 py-1 text-14"
    >
      <Link2 aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 break-words">
        {connection ? (
          <>
            <span className="font-medium">
              {connection.catalog_service_name ?? connection.label}
            </span>
            {" · "}
            {connection.label}
            {" · "}
            {composerConnectionOwnerLabel(connection)}
          </>
        ) : (
          translateUi("Selected connection")
        )}
        <span
          role="status"
          className={
            quietChecking ? "sr-only" : !checking && statusReason ? "text-destructive" : "text-muted-foreground"
          }
        >
          {checking && announceChecking ? (
            <>{" · "}{translateUi("Checking availability…")}</>
          ) : statusReason ? (
            <>
              {translateUi(
                " · Unavailable — choose another connection or remove",
              )}
              {" · "}
              {statusReason}
            </>
          ) : (
            translateUi(" · This message")
          )}
        </span>
        {quietChecking && <span aria-hidden="true" className="text-muted-foreground">
          {" · "}{translateUi("Checking availability…")}
        </span>}
      </span>
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="shrink-0"
        ref={changeButtonRef}
        aria-label={translateUi("Change connection for this message")}
        disabled={disabled}
        onClick={onChange}
      >
        {translateUi("Change")}
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon"
        className="size-7 shrink-0"
        aria-label={translateUi("Remove connection for this message")}
        disabled={disabled}
        onClick={onRemove}
      >
        <X aria-hidden className="size-3.5" />
      </Button>
    </div>
  );
}
