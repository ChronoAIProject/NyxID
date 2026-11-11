#!/usr/bin/env python3
"""Render only validated deployment values; never envsubst nginx/CSP fragments."""
import argparse
import ipaddress
import os
from pathlib import Path
import re
import sys
from urllib.parse import urlsplit


def origin(value: str, name: str) -> str:
    if not value or len(value) > 512 or re.search(r"[\s\x00-\x1f\x7f\\\"';${}<>?#]", value):
        raise ValueError(f"{name} must be an HTTP(S) origin")
    parsed = urlsplit(value)
    if (parsed.scheme not in ("http", "https") or not parsed.hostname
            or parsed.username is not None or parsed.password is not None
            or parsed.path not in ("", "/") or parsed.query or parsed.fragment):
        raise ValueError(f"{name} must contain only scheme, host and optional port")
    host = parsed.hostname
    try:
        address = ipaddress.ip_address(host)
        host = f"[{address}]" if address.version == 6 else str(address)
    except ValueError:
        if (len(host) > 253 or not re.fullmatch(r"[a-zA-Z0-9.-]+", host)
                or any(not re.fullmatch(r"[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?", label)
                       for label in host.split("."))):
            raise ValueError(f"{name} has an invalid host") from None
        host = host.lower()
    port = parsed.port  # Reject malformed/out-of-range ports, including empty ports below.
    if parsed.netloc.endswith(":") or port == 0:
        raise ValueError(f"{name} has an empty port")
    suffix = f":{port}" if port is not None and port != (443 if parsed.scheme == "https" else 80) else ""
    return f"{parsed.scheme}://{host}{suffix}"


def path_literal(value: Path) -> str:
    # Paths are administrator CLI inputs, never HTTP or environment policy text.
    value = value.resolve()
    if not re.fullmatch(r"[a-zA-Z0-9_./ -]+", str(value)):
        raise ValueError("nginx paths must contain only ordinary path characters")
    return f'"{value}"'


def render(env: dict[str, str], template: str, output: Path, root: Path,
           legal_cache: Path, port: int = 8080) -> dict[str, str]:
    backend = origin(env.get("BACKEND_URL", "http://backend:3001"), "BACKEND_URL")
    public = origin(env.get("NYXID_PUBLIC_ORIGIN", "http://localhost:8080"), "NYXID_PUBLIC_ORIGIN")
    extra = env.get("NYXID_BROWSER_CONNECT_ORIGINS", "").split()
    if len(extra) > 16 or len(env.get("NYXID_BROWSER_CONNECT_ORIGINS", "")) > 8192:
        raise ValueError("NYXID_BROWSER_CONNECT_ORIGINS exceeds its bounded origin list")
    connect = {
        "'self'", "https://api.telegram.org", "https://aevatar-console-backend-api.aevatar.ai",
        "https://www.facebook.com", "https://connect.facebook.net", "https://graph.facebook.com",
        "https://assets.unicorn.studio", "https://storage.googleapis.com/unicornstudio-production/",
        "https://oauth.telegram.org",
    }
    # Explicit self WS sources are necessary in browsers that do not consistently
    # match 'self' to the WebSocket scheme. The trusted public origin supplies it.
    for item in [public, *extra]:
        item = origin(item, "NYXID_BROWSER_CONNECT_ORIGINS")
        connect.add(item)
        connect.add(item.replace("https:", "wss:", 1).replace("http:", "ws:", 1))
    csp = "; ".join([
        "default-src 'none'", "base-uri 'self'", "object-src 'none'", "frame-ancestors 'none'",
        "form-action 'self'", "script-src 'self' https://connect.facebook.net https://telegram.org/js/telegram-widget.js https://cdn.jsdelivr.net/gh/hiunicornstudio/unicornstudio.js@v2.1.4/dist/",
        "script-src-attr 'none'", "style-src 'self' 'unsafe-inline' https://fonts.googleapis.com",
        "font-src 'self' https://fonts.gstatic.com", "img-src 'self' https: data: blob:",
        "media-src 'self' blob:", "worker-src 'self'", "manifest-src 'self'",
        "frame-src https://www.facebook.com https://web.facebook.com https://oauth.telegram.org",
        "connect-src " + " ".join(sorted(connect)),
    ])
    common = [
        'add_header X-Frame-Options "DENY" always;',
        'add_header X-Content-Type-Options "nosniff" always;',
        # Only static deployment configuration enables HSTS. Neither Host nor
        # X-Forwarded-Proto participates. Empty nginx header values are omitted.
        'add_header Strict-Transport-Security "' + ("max-age=31536000" if public.startswith("https:") else "") + '" always;',
    ]
    static = common + [
        # All SPA delivery is conservative, including normalized/encoded and
        # trailing-slash routes before JavaScript strips sensitive parameters.
        'add_header Referrer-Policy "no-referrer" always;',
        'add_header Permissions-Policy "camera=(), microphone=(self), geolocation=(), interest-cohort=()" always;',
        f'add_header Content-Security-Policy "{csp}" always;',
    ]
    proxy = common + [
        'proxy_hide_header Strict-Transport-Security;', 'proxy_hide_header X-Frame-Options;',
        'proxy_hide_header X-Content-Type-Options;', 'proxy_hide_header Referrer-Policy;',
        'proxy_hide_header Permissions-Policy;', 'proxy_hide_header Content-Security-Policy;',
        'add_header Referrer-Policy $nyxid_proxy_referrer always;',
        'add_header Permissions-Policy $nyxid_proxy_permissions always;',
        'add_header Content-Security-Policy $nyxid_proxy_csp always;',
    ]
    values = {
        "@@BACKEND_URL@@": backend,
        "@@STATIC_HEADERS@@": path_literal(output / "spa-headers.conf"),
        "@@PROXY_HEADERS@@": path_literal(output / "proxy-headers.conf"),
        "@@ROOT@@": path_literal(root), "@@LEGAL_CACHE@@": path_literal(legal_cache),
        "@@PORT@@": str(port),
    }
    for marker, value in values.items():
        template = template.replace(marker, value)
    if "@@" in template or not 1024 <= port <= 65535:
        raise ValueError("invalid template marker or listen port")
    return {"default.conf": template, "spa-headers.conf": "\n".join(static) + "\n",
            "proxy-headers.conf": "\n".join(proxy) + "\n"}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--template", type=Path, default=Path("/etc/nginx/nyxid/default.conf.template"))
    parser.add_argument("--output-dir", type=Path, default=Path("/etc/nginx/nyxid/generated"))
    parser.add_argument("--root", type=Path, default=Path("/usr/share/nginx/html"))
    parser.add_argument("--legal-cache", type=Path, default=Path("/etc/nginx/legal-cache.conf"))
    parser.add_argument("--port", type=int, default=8080)
    args = parser.parse_args()
    # Validate everything before writing a single output file.
    files = render(dict(os.environ), args.template.read_text(), args.output_dir,
                   args.root, args.legal_cache, args.port)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    for name, value in files.items():
        target = args.output_dir / name
        temporary = target.with_suffix(".tmp")
        temporary.write_text(value)
        temporary.replace(target)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError) as error:
        print(f"NyxID nginx configuration rejected: {error}", file=sys.stderr)
        sys.exit(1)
