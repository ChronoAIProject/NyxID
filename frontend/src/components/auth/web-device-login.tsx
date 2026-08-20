import { useEffect, useState } from "react";
import { useNavigate } from "@tanstack/react-router";
import QRCode from "qrcode";
import {
  Check,
  Clock3,
  Copy,
  ExternalLink,
  QrCode,
  RefreshCw,
  Smartphone,
  X,
} from "lucide-react";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  formatAuthDeviceUserCodeInput,
} from "@/schemas/auth-device";
import { isTrustedAuthReturnTo } from "@/lib/return-url";
import { copyToClipboard } from "@/lib/utils";
import { useWebAuthDeviceLogin } from "@/hooks/use-auth-device";

interface WebDeviceLoginProps {
  readonly returnTo?: string;
}

export function WebDeviceLogin({ returnTo }: WebDeviceLoginProps) {
  const navigate = useNavigate();
  const [isOpen, setIsOpen] = useState(false);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const deviceLogin = useWebAuthDeviceLogin();

  useEffect(() => {
    let cancelled = false;
    const payload = deviceLogin.request?.verification_uri_complete;
    if (!payload) {
      return () => {
        cancelled = true;
      };
    }

    void QRCode.toDataURL(payload, {
      errorCorrectionLevel: "M",
      margin: 2,
      width: 208,
      color: { dark: "#e8e4f0", light: "#0c0b14" },
    }).then((dataUrl) => {
      if (!cancelled) setQrDataUrl(dataUrl);
    }).catch(() => {
      if (!cancelled) setQrDataUrl(null);
    });

    return () => {
      cancelled = true;
    };
  }, [deviceLogin.request?.verification_uri_complete]);

  useEffect(() => {
    if (deviceLogin.phase !== "success") return;
    if (isTrustedAuthReturnTo(returnTo)) {
      window.location.assign(returnTo);
      return;
    }
    void navigate({ to: "/dashboard" as string });
  }, [deviceLogin.phase, navigate, returnTo]);

  function openPanel() {
    setIsOpen(true);
    setCopied(false);
    deviceLogin.start();
  }

  function closePanel() {
    setIsOpen(false);
    setCopied(false);
    deviceLogin.close();
  }

  function generateNew() {
    setCopied(false);
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
        type="button"
        onClick={openPanel}
        className="mt-6 flex h-[44px] w-full cursor-pointer items-center justify-center gap-2.5 rounded-lg border border-border bg-transparent text-[13px] font-medium text-foreground transition-colors duration-200 hover:border-hairline-strong hover:bg-overlay"
      >
        <Smartphone className="size-4 text-nyx-secondary-400" />
        Sign in with the NyxID app
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
    <section className="mt-6 rounded-lg border border-border bg-background p-4" aria-label="NyxID app sign-in">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="flex items-center gap-2 text-[15px] font-semibold">
            <QrCode className="size-4 text-nyx-secondary-400" />
            Sign in with the NyxID app
          </h2>
          <p className="mt-1 text-[12px] text-muted-foreground">
            Approve this browser from a phone that is already signed in.
          </p>
        </div>
        <button
          type="button"
          onClick={closePanel}
          className="flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-overlay hover:text-foreground"
          aria-label="Close NyxID app sign-in"
        >
          <X className="size-4" />
        </button>
      </div>

      {deviceLogin.phase === "requesting" && (
        <p className="mt-5 text-center text-[12px] text-muted-foreground">
          Generating a sign-in code...
        </p>
      )}

      {deviceLogin.request && pending && (
        <div className="mt-5 space-y-4">
          <div className="flex justify-center">
            {qrDataUrl ? (
              <img
                src={qrDataUrl}
                alt="Scan this QR code with the NyxID app"
                className="size-[208px] rounded-md border border-border"
              />
            ) : (
              <div className="flex size-[208px] items-center justify-center rounded-md border border-border text-[12px] text-muted-foreground">
                Preparing QR code...
              </div>
            )}
          </div>

          <div className="text-center">
            <p className="text-[11px] uppercase tracking-[1.5px] text-text-tertiary">
              Enter this code if you open the page manually
            </p>
            <div className="mt-1 flex items-center justify-center gap-2">
              <span className="font-mono text-[24px] font-semibold tracking-[0.18em] text-foreground">
                {formattedCode}
              </span>
              <button
                type="button"
                onClick={() => void copyCode()}
                className="flex size-7 cursor-pointer items-center justify-center rounded-md text-muted-foreground hover:bg-overlay hover:text-foreground"
                aria-label={copied ? "Code copied" : "Copy sign-in code"}
              >
                {copied ? <Check className="size-3.5 text-success" /> : <Copy className="size-3.5" />}
              </button>
            </div>
          </div>

          <p className="text-center text-[12px] leading-relaxed text-muted-foreground">
            Scan with the NyxID app, or open{" "}
            <span className="font-mono text-[11px] text-foreground">{deviceLogin.request.verification_uri}</span>{" "}
            on another device and enter this code.
          </p>

          <div className="flex items-center justify-center gap-1.5 text-[11px] text-warning">
            <Clock3 className="size-3.5" />
            Expires in {String(deviceLogin.remainingSeconds ?? deviceLogin.request.expires_in)}s
          </div>
        </div>
      )}

      {terminalMessage && (
        <div className="mt-5 rounded-md border border-destructive/30 bg-destructive/10 p-3 text-center">
          <p className="text-[13px] font-medium text-destructive">{terminalMessage}</p>
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
        <div className="mt-5 flex items-center justify-center gap-2 text-[13px] text-success">
          <Check className="size-4" />
          Signed in. Redirecting...
        </div>
      )}

      {deviceLogin.phase === "error" && deviceLogin.error?.code === 11206 && (
        <p className="mt-3 flex items-center justify-center gap-1 text-[11px] text-muted-foreground">
          <ExternalLink className="size-3" />
          Please wait before generating another code.
        </p>
      )}
    </section>
  );
}
