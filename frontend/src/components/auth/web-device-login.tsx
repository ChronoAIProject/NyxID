import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import QRCode from "qrcode";
import {
  AlertTriangle,
  ArrowLeft,
  Check,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Copy,
  RefreshCw,
  ScanLine,
} from "lucide-react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useMediaQuery } from "@/hooks/use-media-query";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter,
  DialogHeader, DialogTitle, DialogTrigger,
} from "@/components/ui/dialog";
import { formatAuthDeviceUserCodeInput, type AuthDeviceRequestResponse } from "@/schemas/auth-device";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import { copyToClipboard } from "@/lib/utils";
import { formatWebAuthDeviceRemaining } from "@/lib/auth-device-time";
import { useWebAuthDeviceLogin, type WebAuthDeviceTransport } from "@/hooks/use-auth-device";
import { LOGIN_PROVIDER_ROW_CLASS } from "./login-provider-row";

interface WebDeviceLoginProps {
  readonly returnTo?: string;
  readonly isOpen?: boolean;
  readonly onOpenChange?: (open: boolean) => void;
  readonly triggerLabel?: string;
  readonly triggerIcon?: ReactNode;
  readonly disabled?: boolean;
  readonly transport?: WebAuthDeviceTransport;
  readonly onVerified?: () => void;
  /** Only LoginPage opts into native QR payloads and mobile handoff confirmation. */
  readonly allowAppHandoff?: boolean;
}

interface HandoffSelection {
  readonly request: AuthDeviceRequestResponse;
  readonly url: string;
}

export function WebDeviceLogin({
  returnTo,
  isOpen: controlledOpen,
  onOpenChange,
  triggerLabel = "Continue with the NyxID app",
  triggerIcon,
  disabled = false,
  transport,
  onVerified,
  allowAppHandoff = false,
}: WebDeviceLoginProps) {
  useUiTranslation();
  const navigate = useNavigate();
  const [internalOpen, setInternalOpen] = useState(false);
  const [qrImage, setQrImage] = useState<{ payload: string; dataUrl: string } | null>(null);
  const [copied, setCopied] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const [handoff, setHandoff] = useState<HandoffSelection | null>(null);
  const deviceLogin = useWebAuthDeviceLogin(transport);
  const verifiedRef = useRef(onVerified);
  useEffect(() => { verifiedRef.current = onVerified; }, [onVerified]);
  const isOpen = controlledOpen ?? internalOpen;
  const appLink = allowAppHandoff ? deviceLogin.request?.app_verification_uri : undefined;
  const isCoarsePointer = useMediaQuery("(pointer: coarse)");
  const qrPayload = appLink ?? deviceLogin.request?.verification_uri_complete;
  const qrDataUrl = qrImage?.payload === qrPayload ? qrImage?.dataUrl : null;
  const canHandoff = Boolean(isOpen && isCoarsePointer && appLink && qrDataUrl &&
    deviceLogin.phase === "pending" && deviceLogin.remainingSeconds !== 0);
  const handoffOpen = canHandoff && handoff !== null &&
    handoff.request === deviceLogin.request && handoff.url === appLink;
  // Retire the selection in this render so eligibility returning later cannot
  // revive it, and a replacement never inherits the old confirmation.
  if (handoff !== null && !handoffOpen) setHandoff(null);

  useEffect(() => {
    let cancelled = false;
    const payload = qrPayload;
    if (!isOpen || !payload) {
      return () => {
        cancelled = true;
      };
    }

    void QRCode.toDataURL(payload, {
      errorCorrectionLevel: "M",
      margin: 4,
      width: 208,
      color: { dark: "#0c0b14", light: "#e8e4f0" },
    }).then((dataUrl) => {
      if (!cancelled) setQrImage({ payload, dataUrl });
    }).catch(() => {
      if (!cancelled) setQrImage(null);
    });

    return () => {
      cancelled = true;
    };
  }, [qrPayload, isOpen]);

  useEffect(() => {
    if (isOpen) headingRef.current?.focus();
  }, [isOpen]);

  useEffect(() => {
    if (deviceLogin.phase !== "success") return;
    if (verifiedRef.current) {
      verifiedRef.current();
      return;
    }
    const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
    if (trustedReturnTo) {
      window.location.assign(trustedReturnTo);
      return;
    }
    void navigate({ to: "/assistant" as string });
  }, [deviceLogin.phase, navigate, returnTo]);

  function setOpen(open: boolean) {
    if (controlledOpen === undefined) setInternalOpen(open);
    onOpenChange?.(open);
  }

  function openPanel() {
    setCopied(false);
    setQrImage(null);
    setOpen(true);
    setHandoff(null);
    deviceLogin.start();
  }

  function closePanel() {
    setHandoff(null);
    setCopied(false);
    setQrImage(null);
    deviceLogin.close();
    setOpen(false);
    window.setTimeout(() => triggerRef.current?.focus(), 0);
  }

  function generateNew() {
    setCopied(false);
    setQrImage(null);
    setHandoff(null);
    deviceLogin.generateNew();
  }

  function isHandoffCurrent(selection: HandoffSelection) {
    return canHandoff && selection.request === deviceLogin.request &&
      selection.url === appLink && selection.url === selection.request.app_verification_uri &&
      window.matchMedia?.("(pointer: coarse)").matches &&
      deviceLogin.isRequestActive(selection.request);
  }

  function changeHandoffOpen(open: boolean) {
    const selection = deviceLogin.request && appLink
      ? { request: deviceLogin.request, url: appLink } : null;
    setHandoff(open && selection && isHandoffCurrent(selection) ? selection : null);
  }

  function confirmHandoff(event: MouseEvent<HTMLAnchorElement>) {
    if (!handoff || !isHandoffCurrent(handoff)) event.preventDefault();
    setHandoff(null);
  }

  async function copyCode() {
    const code = deviceLogin.request?.user_code;
    if (!code) return;
    await copyToClipboard(formatAuthDeviceUserCodeInput(code));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  if (!isOpen) {
    return (
      <button
        ref={triggerRef}
        type="button"
        onClick={openPanel}
        disabled={disabled}
        className={LOGIN_PROVIDER_ROW_CLASS}
        aria-label={triggerLabel}
      >
        {triggerIcon ?? (
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-overlay-strong">
            <NyxidIcon alt="" className="h-4 w-4 object-contain" />
          </span>
        )}
        {triggerLabel}
        <ChevronRight className="ml-auto size-4 text-muted-foreground" />
      </button>
    );
  }

  const terminalMessage =
    deviceLogin.phase === "denied"
      ? "Sign-in was rejected"
      : deviceLogin.phase === "expired"
        ? "Code expired - generate a new one"
        : deviceLogin.phase === "used"
          ? "This code was already used"
          : deviceLogin.phase === "error" || deviceLogin.phase === "restore_error"
            ? deviceLogin.error?.message ?? "Device sign-in failed"
            : null;
  const formattedCode = deviceLogin.request
    ? formatAuthDeviceUserCodeInput(deviceLogin.request.user_code)
    : null;
  const pending = deviceLogin.phase === "pending";

  return (
    <section aria-labelledby="nyxid-app-login-heading">
      <button
        type="button"
        onClick={closePanel}
        className="mb-5 flex cursor-pointer items-center gap-1.5 type-row-title unthemed:text-12 text-muted-foreground transition-colors hover:text-foreground"
        aria-label={translateUi("Back to all sign-in options")}
      >
        <ArrowLeft className="size-3.5" />{translateUi("Back to all sign-in options")}</button>

      <div className="text-center">
        <h2
          id="nyxid-app-login-heading"
          ref={headingRef}
          tabIndex={-1}
          className="type-section-heading outline-none"
        >{translateUi("Continue with the NyxID app")}</h2>
        <p className="mx-auto mt-1.5 max-w-[340px] text-14 text-muted-foreground">{translateUi("Approve this sign-in from a phone that's already signed in to NyxID.")}</p>
      </div>

      {deviceLogin.phase === "requesting" && (
        <div
          className="mt-8 flex items-center justify-center gap-2 text-14 unthemed:text-12 text-muted-foreground"
          role="status"
        >
          <RefreshCw className="size-3.5 animate-spin" />{translateUi("Generating a sign-in code...")}</div>
      )}

      {deviceLogin.phase === "restoring" && (
        <div className="mt-8 flex items-center justify-center gap-2 text-14 unthemed:text-12 text-muted-foreground" role="status">
          <RefreshCw className="size-3.5 animate-spin" />{translateUi("Loading your approved account...")}</div>
      )}

      {pending && (!deviceLogin.request || deviceLogin.remainingSeconds === 0) && (
        <div className="mt-8 flex items-center justify-center gap-2 text-14 unthemed:text-12 text-muted-foreground" role="status">
          <RefreshCw className="size-3.5 animate-spin" />{translateUi("Checking your request…")}</div>
      )}

      {deviceLogin.request && pending && deviceLogin.remainingSeconds !== 0 && (
        <div className="mt-6 space-y-5">
          <div className="flex justify-center">
            <Dialog open={handoffOpen} onOpenChange={changeHandoffOpen}>
              <QrFrame interactive={canHandoff}>
                {qrDataUrl ? (
                  <img
                    src={qrDataUrl}
                    alt={translateUi("QR code to continue with the NyxID app")}
                    className="size-[208px] rounded-md object-contain"
                  />
                ) : (
                  <div className="flex size-[208px] items-center justify-center gap-2 text-14 unthemed:text-12 text-muted-foreground">
                    <ScanLine className="size-4" />{translateUi("Preparing QR code...")}</div>
                )}
              </QrFrame>
              {handoffOpen && handoff && (
                <DialogContent>
                  <DialogHeader>
                    <DialogTitle>{translateUi("Open NyxID app?")}</DialogTitle>
                    <DialogDescription>{translateUi("Open this login in the app without scanning. Review and approve it, then return to this browser tab.")}</DialogDescription>
                  </DialogHeader>
                  <DialogFooter>
                    <DialogClose asChild><Button variant="outline">{translateUi("Cancel")}</Button></DialogClose>
                    <Button asChild variant="primary" className="flex-1 whitespace-normal md:flex-none">
                      <a
                        href={handoff.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        referrerPolicy="no-referrer"
                        onClick={confirmHandoff}
                      >
                        {translateUi("Open NyxID app")}
                      </a>
                    </Button>
                  </DialogFooter>
                </DialogContent>
              )}
            </Dialog>
          </div>

          {canHandoff && <p className="text-center type-secondary text-muted-foreground">{translateUi("Tap the QR code to open NyxID, or scan it with another device.")}</p>}

          <div className="text-center">
            <p className="text-13 unthemed:text-11 uppercase text-text-tertiary">{translateUi("Manual code")}</p>
            <div className="mt-1.5 flex items-center justify-center gap-2">
              <span className="font-mono type-section-heading text-foreground">
                {formattedCode}
              </span>
              <button
                type="button"
                onClick={() => void copyCode()}
                className="flex size-8 cursor-pointer items-center justify-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground"
                aria-label={copied ? translateUi("Sign-in code copied") : translateUi("Copy sign-in code")}
              >
                {copied ? (
                  <Check className="size-3.5 text-success" />
                ) : (
                  <Copy className="size-3.5" />
                )}
              </button>
            </div>
          </div>

          <p className="text-center text-14 unthemed:text-12 text-muted-foreground">{translateUi("Scan the code, or open")}{" "}
            <span className="break-all font-mono text-13 unthemed:text-11 text-foreground">
              {deviceLogin.request.verification_uri}
            </span>{" "}{translateUi("on your phone and enter the manual code.")}</p>

          <div className="flex items-center justify-center gap-1.5 text-13 unthemed:text-11 text-warning">
            <Clock3 className="size-3.5" />{translateUi("Expires in")}{" "}
            {formatWebAuthDeviceRemaining(
              deviceLogin.remainingSeconds ?? deviceLogin.request.expires_in,
            )}
          </div>
        </div>
      )}

      {terminalMessage && (
        <div
          className="mt-6 rounded-md border border-destructive/30 bg-destructive/10 p-4 text-center"
          role="status"
        >
          <AlertTriangle className="mx-auto size-5 text-destructive" />
          <p className="mt-2 type-card-title unthemed:text-13 text-destructive">
            {translateUi(terminalMessage)}
          </p>
          <Button
            type="button"
            variant="outline"
            className="mt-3"
            onClick={deviceLogin.phase === "restore_error" ? deviceLogin.retryProfile : generateNew}
            disabled={deviceLogin.phase === "requesting"}
          >
            <ButtonIcon>
              <RefreshCw />
            </ButtonIcon>
            {deviceLogin.phase === "restore_error" ? translateUi("Retry loading account") : translateUi("Generate new code")}
          </Button>
        </div>
      )}

      {deviceLogin.phase === "restricted" && deviceLogin.loginCode && (
        <section className="mt-6 space-y-3 text-center" aria-live="polite">
          <CheckCircle2 className="mx-auto size-6 text-success" />
          <h3 className="type-card-title">{translateUi("Restricted terminal login")}</h3>
          <p className="text-14 unthemed:text-12 text-muted-foreground">{translateUi("This browser remains signed out.")}</p>
          <code data-sensitive className="block font-mono text-14">{deviceLogin.loginCode.code}</code>
          <p className="text-14 unthemed:text-12">{translateUi("Terminal command: ")}<code>nyxid login --code</code></p>
          <p className="text-14 unthemed:text-12 text-muted-foreground">{translateUi("Expires ")}{new Date(deviceLogin.loginCode.expires_at).toLocaleTimeString(getFormattingLocale())}</p>
        </section>
      )}

      {deviceLogin.phase === "success" && (
        <div
          className="mt-8 flex items-center justify-center gap-2 text-15 unthemed:text-13 text-success"
          role="status"
        >
          <CheckCircle2 className="size-4" />
          {onVerified ? translateUi("Identity verified") : translateUi("Signed in. Redirecting...")}
        </div>
      )}
    </section>
  );
}

function QrFrame({ interactive, children }: {
  readonly interactive: boolean;
  readonly children: ReactNode;
}) {
  const className = "relative flex size-[216px] items-center justify-center rounded-lg border border-border bg-[#0c0b14] p-1";
  return interactive ? (
    <DialogTrigger asChild>
      <button type="button" className={`${className} cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring`} aria-label={translateUi("Open NyxID app from QR code")}>
        {children}
      </button>
    </DialogTrigger>
  ) : (
    <div className={className}>{children}</div>
  );
}
