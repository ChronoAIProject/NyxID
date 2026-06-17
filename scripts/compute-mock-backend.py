#!/usr/bin/env python3
"""Small OpenAI-compatible mock backend for NyxID compute pool QA.

The server intentionally does not log request bodies, prompts, responses, or
Authorization values. Use it to test NyxID queueing, worker polling, routing,
result handling, cancellation, and token separation before using a real GPU.
"""

from __future__ import annotations

import argparse
import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Handler(BaseHTTPRequestHandler):
    server_version = "nyxid-compute-mock/0.1"

    def log_message(self, fmt: str, *args: object) -> None:
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def do_GET(self) -> None:
        if self.path == "/health":
            self._send_json({"status": "ok", "name": self.server.name})
            return
        self._send_json({"error": "not found"}, status=404)

    def do_POST(self) -> None:
        if self.server.required_token:
            auth = self.headers.get("authorization", "")
            if auth != f"Bearer {self.server.required_token}":
                self._send_json({"error": "unauthorized"}, status=401)
                return

        length = int(self.headers.get("content-length", "0"))
        if length > self.server.max_body_bytes:
            self._drain(length)
            self._send_json({"error": "request too large"}, status=413)
            return

        raw = self.rfile.read(length)
        try:
            body = json.loads(raw or b"{}")
        except json.JSONDecodeError:
            self._send_json({"error": "invalid json"}, status=400)
            return

        if self.server.delay_secs:
            time.sleep(self.server.delay_secs)

        if self.server.fail_status:
            self._send_json(
                {"error": f"forced failure from {self.server.name}"},
                status=self.server.fail_status,
            )
            return

        model = body.get("model", "unknown-model") if isinstance(body, dict) else "unknown-model"
        response = {
            "id": f"mock-{self.server.name}",
            "object": "chat.completion",
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": f"hello from {self.server.name}",
                    },
                    "finish_reason": "stop",
                }
            ],
            "usage": {
                "prompt_tokens": 1,
                "completion_tokens": 4,
                "total_tokens": 5,
            },
        }
        self._send_json(response)

    def _drain(self, length: int) -> None:
        remaining = length
        while remaining > 0:
            chunk = self.rfile.read(min(remaining, 65536))
            if not chunk:
                break
            remaining -= len(chunk)

    def _send_json(self, payload: dict, status: int = 200) -> None:
        data = json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)


class Server(ThreadingHTTPServer):
    def __init__(
        self,
        address: tuple[str, int],
        name: str,
        delay_secs: float,
        fail_status: int | None,
        required_token: str | None,
        max_body_bytes: int,
    ) -> None:
        super().__init__(address, Handler)
        self.name = name
        self.delay_secs = delay_secs
        self.fail_status = fail_status
        self.required_token = required_token
        self.max_body_bytes = max_body_bytes


def main() -> None:
    parser = argparse.ArgumentParser(description="NyxID compute mock backend")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8001)
    parser.add_argument("--name", default="mock-worker")
    parser.add_argument("--delay-secs", type=float, default=0.0)
    parser.add_argument("--fail-status", type=int)
    parser.add_argument("--require-token-env")
    parser.add_argument("--max-body-bytes", type=int, default=8 * 1024 * 1024)
    args = parser.parse_args()

    required_token = None
    if args.require_token_env:
        import os

        required_token = os.environ.get(args.require_token_env)
        if not required_token:
            raise SystemExit(f"{args.require_token_env} is empty or unset")

    server = Server(
        (args.host, args.port),
        args.name,
        args.delay_secs,
        args.fail_status,
        required_token,
        args.max_body_bytes,
    )
    print(
        f"mock backend {args.name} listening on http://{args.host}:{args.port}; "
        "request bodies and tokens are not logged",
        file=sys.stderr,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
