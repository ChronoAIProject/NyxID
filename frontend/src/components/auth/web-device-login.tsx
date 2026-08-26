import { useEffect, useRef, useState } from "react";
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
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { Button, ButtonIcon } from "@/components/ui/button";
import { formatAuthDeviceUserCodeInput } from "@/schemas/auth-device";
import { resolveTrustedAuthReturnTo } from "@/lib/return-url";
import { copyToClipboard } from "@/lib/utils";
import { formatWebAuthDeviceRemaining } from "@/lib/auth-device-time";
import { useWebAuthDeviceLogin } from "@/hooks/use-auth-device";

interface WebDeviceLoginProps {
  readonly returnTo?: string;
  readonly isOpen?: boolean;
  readonly onOpenChange?: (open: boolean) => void;
}

export const LOGIN_PROVIDER_ROW_CLASS =
  "flex h-[46px] w-full cursor-pointer items-center gap-3 rounded-lg border border-border bg-background px-4 text-[13.5px] font-medium text-foreground transition-colors duration-300 hover:border-border/80 hover:bg-overlay active:scale-[0.99]";

export function WebDeviceLogin({
  returnTo,
  isOpen: controlledOpen,
  onOpenChange,
}: WebDeviceLoginProps) {
  const navigate = useNavigate();
  const [internalOpen, setInternalOpen] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const deviceLogin = useWebAuthDeviceLogin();
  const isOpen = controlledOpen ?? internalOpen;

  useEffect(() => {
    let cancelled = false;
    const payload = deviceLogin.request?.verification_uri_complete;
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
      if (!cancelled) setQrDataUrl(dataUrl);
    }).catch(() => {
      if (!cancelled) setQrDataUrl(null);
    });

    return () => {
      cancelled = true;
    };
  }, [deviceLogin.request?.verification_uri_complete, isOpen]);

  useEffect(() => {
    if (isOpen) headingRef.current?.focus();
  }, [isOpen]);

  useEffect(() => {
    if (deviceLogin.phase !== "success") return;
    const trustedReturnTo = resolveTrustedAuthReturnTo(returnTo);
    if (trustedReturnTo) {
      window.location.assign(trustedReturnTo);
      return;
    }
    void navigate({ to: "/dashboard" as string });
  }, [deviceLogin.phase, navigate, returnTo]);

  function setOpen(open: boolean) {
    if (controlledOpen === undefined) setInternalOpen(open);
    onOpenChange?.(open);
  }

  function openPanel() {
    setCopied(false);
    setQrDataUrl(null);
    setOpen(true);
    deviceLogin.start();
  }

  function closePanel() {
    setCopied(false);
    setQrDataUrl(null);
    deviceLogin.close();
    setOpen(false);
    window.setTimeout(() => triggerRef.current?.focus(), 0);
  }

  function generateNew() {
    setCopied(false);
    setQrDataUrl(null);
    deviceLogin.generateNew();
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
        className={LOGIN_PROVIDER_ROW_CLASS}
        aria-label="Continue with the NyxID app"
      >
        <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[8px] bg-overlay-strong">
          <NyxidIcon alt="" className="h-4 w-4 object-contain" />
        </span>
        Continue with the NyxID app
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
          : deviceLogin.phase === "error"
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
        className="mb-5 flex cursor-pointer items-center gap-1.5 text-[12px] font-medium text-muted-foreground transition-colors hover:text-foreground"
        aria-label="Back to all sign-in options"
      >
        <ArrowLeft className="size-3.5" />
        Back to all sign-in options
      </button>

      <div className="text-center">
        <h2
          id="nyxid-app-login-heading"
          ref={headingRef}
          tabIndex={-1}
          className="text-[20px] font-semibold outline-none"
        >
          Continue with the NyxID app
        </h2>
        <p className="mx-auto mt-1.5 max-w-[340px] text-[12.5px] leading-relaxed text-muted-foreground">
          Approve this sign-in from a phone that&apos;s already signed in to
          NyxID.
        </p>
      </div>

      {deviceLogin.phase === "requesting" && (
        <div
          className="mt-8 flex items-center justify-center gap-2 text-[12px] text-muted-foreground"
          role="status"
        >
          <RefreshCw className="size-3.5 animate-spin" />
          Generating a sign-in code...
        </div>
      )}

      {deviceLogin.request && pending && (
        <div className="mt-6 space-y-5">
          <div className="flex justify-center">
            <div className="relative flex size-[216px] items-center justify-center rounded-lg border border-border bg-[#0c0b14] p-1">
              {qrDataUrl ? (
                <img
                  src={qrDataUrl}
                  alt="QR code to continue with the NyxID app"
                  className="size-[208px] rounded-md object-contain"
                />
              ) : (
                <div className="flex size-[208px] items-center justify-center gap-2 text-[12px] text-muted-foreground">
                  <ScanLine className="size-4" />
                  Preparing QR code...
                </div>
              )}
            </div>
          </div>

          <div className="text-center">
            <p className="text-[11px] uppercase text-text-tertiary">
              Manual code
            </p>
            <div className="mt-1.5 flex items-center justify-center gap-2">
              <span className="font-mono text-[24px] font-semibold text-foreground">
                {formattedCode}
              </span>
              <button
                type="button"
                onClick={() => void copyCode()}
                className="flex size-8 cursor-pointer items-center justify-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-overlay hover:text-foreground"
                aria-label={copied ? "Sign-in code copied" : "Copy sign-in code"}
              >
                {copied ? (
                  <Check className="size-3.5 text-success" />
                ) : (
                  <Copy className="size-3.5" />
                )}
              </button>
            </div>
          </div>

          <p className="text-center text-[12px] leading-relaxed text-muted-foreground">
            Scan the code, or open{" "}
            <span className="break-all font-mono text-[11px] text-foreground">
              {deviceLogin.request.verification_uri}
            </span>{" "}
            on your phone and enter the manual code.
          </p>

          <div className="flex items-center justify-center gap-1.5 text-[11px] text-warning">
            <Clock3 className="size-3.5" />
            Expires in{" "}
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
          <p className="mt-2 text-[13px] font-medium text-destructive">
            {terminalMessage}
          </p>
          <Button
            type="button"
            variant="outline"
            className="mt-3"
            onClick={generateNew}
            disabled={deviceLogin.phase === "requesting"}
          >
            <ButtonIcon>
              <RefreshCw />
            </ButtonIcon>
            Generate new code
          </Button>
        </div>
      )}

      {deviceLogin.phase === "success" && (
        <div
          className="mt-8 flex items-center justify-center gap-2 text-[13px] text-success"
          role="status"
        >
          <CheckCircle2 className="size-4" />
          Signed in. Redirecting...
        </div>
      )}
    </section>
  );
}
