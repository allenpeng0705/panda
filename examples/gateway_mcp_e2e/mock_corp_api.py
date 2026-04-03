#!/usr/bin/env python3
"""Minimal HTTP/1.1 mock corporate API for gateway_mcp_e2e (GET /allowed/toolpath -> JSON)."""
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import sys


class Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:
        if self.path.split("?", 1)[0] == "/allowed/toolpath":
            body = json.dumps({"ok": True, "via": "mock_corp_api"}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(404)

    def log_message(self, fmt: str, *args) -> None:
        pass


def main() -> None:
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 18081
    server = HTTPServer(("127.0.0.1", port), Handler)
    print(f"mock_corp_api listening on http://127.0.0.1:{port}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
