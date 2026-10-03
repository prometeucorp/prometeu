#!/usr/bin/env python3
"""Synthetic provider for native WebView2 -> Tauri -> WSL acceptance checks."""
import json
import re
from pathlib import Path
import sys
import time
import subprocess
import tomllib

turn_number = 0
turn_id = ""
item_id = ""

launched = False


def send(frame):
    print(json.dumps(frame), flush=True)


def complete(text):
    send({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": item_id, "text": text}}})
    send({"method": "turn/completed", "params": {"turn": {"id": turn_id, "status": "completed"}}})


for line in sys.stdin:
    request = json.loads(line)
    if request.get("id") == "native-approval" and "result" in request:
        assert request["result"]["decision"] == "accept"
        complete("Native approval received")
        continue
    method = request.get("method")
    params = request.get("params", {})
    result = {}
    if method == "account/read":
        result = {"account": {"type": "chatgpt", "email": "fixture@example.test", "planType": "test"}}
    elif method == "model/list":
        result = {"data": [{"model": "native-fixture", "displayName": "Native fixture", "supportedReasoningEfforts": [{"reasoningEffort": "high"}]}], "nextCursor": None}
    elif method in ("thread/start", "thread/resume"):
        if not launched:
            with Path("provider-launches").open("a") as launches:
                launches.write("launch\n")
            launched = True
        if method == "thread/resume":
            assert params["threadId"] == "native-thread"
        result = {"thread": {"id": "native-thread"}, "model": "native-fixture"}
    elif method == "turn/start":
        turn_number += 1
        turn_id = f"native-turn-{time.time_ns()}-{turn_number}"
        item_id = f"answer-{turn_id}"
        result = {"turn": {"id": turn_id}}
    if "id" in request:
        send({"id": request["id"], "result": result})
    if method != "turn/start":
        continue
    text = params["input"][0]["text"]
    with Path("provider-inputs").open("a") as inputs:
        inputs.write(json.dumps(text) + "\n")
    send({"method": "item/started", "params": {"item": {"type": "agentMessage", "id": item_id}}})
    if text.endswith("Read native attachment"):
        match = re.match(r'@(?:"([^"]+)"|(\S+))', text)
        assert match, text
        path = Path(match.group(1) or match.group(2))
        complete("Native attachment: " + path.read_text())
        continue
    if text == "Check native tools":
        overrides = [arg for arg in sys.argv if arg.startswith("mcp_servers=")]
        server = tomllib.loads(overrides[0])["mcp_servers"]["native-check"]
        result = subprocess.run([server["command"], *server.get("args", [])], check=True, capture_output=True, text=True)
        complete(result.stdout.strip())
        continue
    if text == "Native approval":
        send({"id": "native-approval", "method": "item/commandExecution/requestApproval", "params": {"itemId": "command", "command": "printf native-approved"}})
        continue
    if text == "Continue while window is closed":
        send({"method": "item/agentMessage/delta", "params": {"itemId": item_id, "delta": "Waiting for native window closure"}})
        while not Path("release-turn").exists():
            time.sleep(0.02)
        complete("Completed after native window closure")
        continue
    if text == "Continue while connection is lost":
        send({"method": "item/agentMessage/delta", "params": {"itemId": item_id, "delta": "Waiting for attachment recovery"}})
        while not Path("release-recovery").exists():
            time.sleep(0.02)
        complete("Completed during attachment recovery")
        continue
    complete(text)
