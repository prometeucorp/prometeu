#!/usr/bin/env python3
"""Hermetic MCP/OAuth server shared by runtime and native Windows acceptance."""
import base64
import hashlib
import json
import pathlib
import os
import time
import sys
import threading
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

root = pathlib.Path(sys.argv[1])
root.mkdir(parents=True, exist_ok=True)
lock = threading.Lock()
state = {"registrations": 0, "exchanges": 0, "refreshes": 0, "authorized": {}, "probes": 0}


def snapshot():
    with lock:
        temporary = root / "stats.tmp"
        temporary.write_text(json.dumps(state))
        temporary.replace(root / "stats.json")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def reply(self, body, status=200, **headers):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for key, value in headers.items():
            self.send_header(key.replace("_", "-"), value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        request = urllib.parse.urlsplit(self.path)
        if request.path == "/authorize":
            query = {k: v[0] for k, v in urllib.parse.parse_qs(request.query).items()}
            assert query["redirect_uri"] == "http://127.0.0.1:17421/mcp"
            assert query["code_challenge_method"] == "S256"
            until = time.monotonic() + 60
            while (root / "hold-consent").exists() and time.monotonic() < until:
                time.sleep(.05)
            state["authorized"][query["state"]] = query
            snapshot()
            self.reply({}, 302, Location=query["redirect_uri"] + "?" + urllib.parse.urlencode({"state": query["state"], "code": query["state"]}))
        elif request.path.startswith("/.well-known/oauth-protected-resource"):
            self.reply({"resource": url + "/mcp", "authorization_servers": [url]})
        elif request.path == "/.well-known/oauth-authorization-server":
            self.reply({"authorization_endpoint": url + "/authorize", "token_endpoint": url + "/token", "registration_endpoint": url + "/register", "scopes_supported": ["read"]})
        else:
            self.reply({}, 404)

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        if self.path == "/register":
            registration = json.loads(body)
            assert registration["redirect_uris"] == ["http://127.0.0.1:17421/mcp"]
            assert registration["token_endpoint_auth_method"] == "none"
            state["registrations"] += 1
            snapshot()
            self.reply({"client_id": "fixture-client"})
        elif self.path == "/token":
            query = {k: v[0] for k, v in urllib.parse.parse_qs(body.decode()).items()}
            if query["grant_type"] == "authorization_code":
                consent = state["authorized"].pop(query["code"], None)
                digest = base64.urlsafe_b64encode(hashlib.sha256(query["code_verifier"].encode()).digest()).decode().rstrip("=")
                if not consent or digest != consent["code_challenge"] or query["resource"] != consent["resource"]:
                    self.reply({"error": "invalid_grant"}, 400)
                    return
                state["exchanges"] += 1
                self.reply({"access_token": "fixture-initial", "refresh_token": "fixture-refresh", "expires_in": 1})
            else:
                assert query["refresh_token"] == "fixture-refresh"
                state["refreshes"] += 1
                self.reply({"access_token": "fixture-renewed", "expires_in": 3600})
            snapshot()
        elif self.path == "/mcp":
            if self.headers.get("Authorization") != "Bearer fixture-renewed":
                self.reply({}, 401, WWW_Authenticate='Bearer resource_metadata="' + url + '/.well-known/oauth-protected-resource/mcp"')
                return
            request = json.loads(body)
            if request["method"] == "initialize":
                state["probes"] += 1
                snapshot()
                self.reply({"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "Fixture MCP", "version": "1"}}})
            elif request["method"] == "tools/list":
                self.reply({"jsonrpc": "2.0", "id": 2, "result": {"tools": [{"name": "fixture", "inputSchema": {"type": "object"}}]}})
            else:
                self.reply({}, 202)
        else:
            self.reply({}, 404)


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
url = f"http://127.0.0.1:{server.server_port}"
snapshot()
(root / "server.pid").write_text(str(os.getpid()))
print(url, flush=True)
server.serve_forever()
