#!/usr/bin/env python3
"""A stand-in for `agent-desktop mcp` in the guard's integration test.

Speaks MCP over stdio, answers from the recorded fixtures beside it, and
appends its argv and every tools/call it gets to $FAKE_AGENT_DESKTOP_LOG
(one JSON object per line), so the test can see what reached it.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOG = os.environ.get("FAKE_AGENT_DESKTOP_LOG")


def log(entry):
    if LOG:
        with open(LOG, "a") as file:
            file.write(json.dumps(entry) + "\n")


def fixture(name):
    with open(os.path.join(HERE, name)) as file:
        return json.load(file)


def ok(command, data):
    envelope = {"version": "2.6", "ok": True, "command": command, "data": data}
    return {
        "content": [{"type": "text", "text": json.dumps(envelope)}],
        "structuredContent": envelope,
        "isError": False,
    }


snapshots = 0


def call(name, args):
    global snapshots
    command = name.removeprefix("desktop_").replace("_", "-")
    if name == "desktop_list_windows":
        return ok(command, fixture("list-windows.json")["data"])
    if name == "desktop_snapshot":
        snapshots += 1
        sid = "sfake%d" % snapshots
        data = {"app": args.get("app", ""), "snapshot_id": sid,
                "tree": {"role": "window", "children": [
                    {"role": "button", "name": "OK", "ref_id": "@%s:e1" % sid}]}}
        if "window_id" in args:
            data["window"] = {"id": args["window_id"], "title": ""}
        return ok(command, data)
    return ok(command, {"disposition": {"delivery": "delivered_verified", "retry": "unsafe"}})


log({"argv": sys.argv[1:]})
for line in sys.stdin:
    try:
        message = json.loads(line)
    except ValueError:
        continue
    method = message.get("method")
    if "id" not in message:
        continue
    if method == "initialize":
        result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                  "serverInfo": {"name": "agent-desktop", "version": "fake"},
                  "instructions": "Fake agent-desktop."}
    elif method == "tools/list":
        result = fixture("mcp-tools.json")
    elif method == "tools/call":
        params = message.get("params", {})
        log({"call": params.get("name"), "arguments": params.get("arguments", {})})
        result = call(params.get("name"), params.get("arguments") or {})
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}) + "\n")
    sys.stdout.flush()
