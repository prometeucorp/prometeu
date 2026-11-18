"""Disposable MCP fixture: metadata receipts only, no model or tool execution."""
import json
import os
from pathlib import Path
import signal
import sys
import threading
import time

receipt, shutdown = map(Path, sys.argv[1:3])
lock = threading.RLock()


def record(value):
    with lock, receipt.open("a") as output:
        output.write(json.dumps(value) + "\n")


def finish(*_):
    record({"event": "exited"})
    os._exit(0)


def watchdog():
    deadline = time.monotonic() + 35
    while time.monotonic() < deadline and not shutdown.exists():
        time.sleep(0.05)
    finish()


os.umask(0o077)
record({"event": "started", "pid": os.getpid()})
for event in (signal.SIGHUP, signal.SIGTERM, signal.SIGINT):
    signal.signal(event, finish)
threading.Thread(target=watchdog, daemon=True).start()
try:
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get("method")
        record({
            "method": method if method in (
                "initialize", "notifications/initialized", "tools/list", "tools/call", "ping"
            ) else "unexpected",
            "environment_matches": os.environ.get("PROMETEU_MCP_SMOKE_VALUE") ==
                "PROMETEU_SELECTED_STDIO_ENVIRONMENT",
        })
        if "id" not in request:
            continue
        if method == "initialize":
            result = {
                "protocolVersion": request.get("params", {}).get("protocolVersion", "2024-11-05"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "prometeu-native-mcp-smoke", "version": "1.0.0"},
            }
        elif method == "tools/list":
            result = {"tools": [{
                "name": "prometeu_native_smoke_marker",
                "description": "Synthetic conformance fixture. Do not invoke.",
                "inputSchema": {"type": "object", "properties": {}},
            }]}
        else:
            result = {}
        print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
finally:
    finish()
