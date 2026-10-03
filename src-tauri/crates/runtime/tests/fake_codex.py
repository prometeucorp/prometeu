#!/usr/bin/env python3
"""Hermetic app-server fixture for testing the actual runtime executable."""
import json
import sys
import subprocess
import time


def send(value):
    print(json.dumps(value), flush=True)


for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    params = request.get("params", {})
    if method == "account/read":
        result = {"account": {"type": "chatgpt", "email": "fixture@example.test", "planType": "test"}}
    elif method == "model/list":
        result = {"data": [{"model": "fixture", "displayName": "Fixture", "supportedReasoningEfforts": [{"reasoningEffort": "high"}]}], "nextCursor": None}
    elif method in ("thread/start", "thread/resume"):
        if method == "thread/resume":
            assert params["threadId"] == "fixture-thread"
        result = {"thread": {"id": "fixture-thread"}, "model": "fixture"}
    elif method == "turn/start":
        turn_id = f"fixture-turn-{time.time_ns()}"
        result = {"turn": {"id": turn_id}}
    else:
        result = {}
    if "id" in request:
        send({"id": request["id"], "result": result})
    if method == "turn/start":
        text = params["input"][0]["text"]
        send({"method": "item/started", "params": {"item": {"type": "agentMessage", "id": "answer"}}})
        send({"method": "item/agentMessage/delta", "params": {"itemId": "answer", "delta": text}})
        send({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "answer", "text": text}}})
        if text == "hold":
            subprocess.Popen([sys.executable, "-c", "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)"])
            send({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "held", "text": "child-started"}}})
            continue
        send({"method": "turn/completed", "params": {"turn": {"id": turn_id, "status": "completed"}}})
