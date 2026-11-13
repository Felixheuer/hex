#!/usr/bin/env python3
"""A stand-in for `cua-driver mcp --direct` in the guard's tests and the
driver update's self-test.

Speaks MCP over stdio with Cua Driver's own tool names and shapes
(`list_windows`, `get_window_state`, `click` by `element_token`, …),
answers from the recorded windows beside it (`list-windows.json`, as the
guard lists them, turned back into Cua's shape), and appends its argv, the
environment Agent-Desk gives it, and every tools/call it gets to
$FAKE_CUA_DRIVER_LOG (one JSON object per line), so a test can see what
reached it. Nothing here touches the desktop.
"""
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LOG = os.environ.get("FAKE_CUA_DRIVER_LOG")


def log(entry):
    if LOG:
        with open(LOG, "a") as file:
            file.write(json.dumps(entry) + "\n")


def windows():
    with open(os.path.join(HERE, "list-windows.json")) as file:
        listed = json.load(file)["data"]
    found = []
    for index, window in enumerate(listed):
        found.append({
            "window_id": int(window["id"].removeprefix("w-")),
            "pid": window.get("pid", 0),
            "app_name": window.get("app_name", ""),
            "title": window.get("title", ""),
            "bounds": window.get("bounds", {}),
            "is_on_screen": window.get("visible", True),
            "layer": 0,
            "z_index": len(listed) - index,
        })
    return found


def structured(value):
    return {"content": [{"type": "text", "text": "ok"}], "structuredContent": value,
            "isError": False}


snapshots = 0


def call(name, args):
    global snapshots
    if name == "list_windows":
        found = windows()
        if args.get("app"):
            found = [window for window in found if window["app_name"] == args["app"]]
        return structured({"windows": found})
    if name == "list_apps":
        apps = {}
        for window in windows():
            apps[window["pid"]] = {"name": window["app_name"], "pid": window["pid"],
                                   "running": True, "bundle_id": None}
        return structured({"apps": list(apps.values())})
    if name == "get_window_state":
        snapshots += 1
        sid = "s%08d" % snapshots
        title = next((window["title"] for window in windows()
                      if window["window_id"] == args.get("window_id")), "")
        app = next((window["app_name"] for window in windows()
                    if window["window_id"] == args.get("window_id")), "")
        return structured({
            "app_name": app, "snapshot_id": sid, "window_id": args.get("window_id"),
            "window_title": title, "truncated": False,
            "elements": [
                {"actions": ["AXRaise"], "depth": 0, "element_index": 0,
                 "element_token": sid + ":0", "label": title, "role": "AXWindow"},
                {"actions": ["AXPress"], "depth": 1, "element_index": 1, "parent_index": 0,
                 "element_token": sid + ":1", "label": "OK", "role": "AXButton"},
            ]})
    return structured({"effect": "confirmed", "route": "accessibility"})


TOOLS = ["list_windows", "list_apps", "get_window_state", "click", "double_click",
         "right_click", "type_text", "press_key", "hotkey", "set_value", "scroll",
         "launch_app", "bring_to_front", "set_window_frame"]

if os.environ.get("FAKE_CUA_DRIVER_BROKEN"):
    sys.exit(3)
log({"argv": sys.argv[1:],
     "env": {key: value for key, value in os.environ.items()
             if key.startswith("CUA_") or key == "DO_NOT_TRACK"}})
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
                  "serverInfo": {"name": "cua-driver", "version": "fake"}}
    elif method == "tools/list":
        result = {"tools": [{"name": name, "inputSchema": {"type": "object"}} for name in TOOLS]}
    elif method == "tools/call":
        params = message.get("params", {})
        log({"call": params.get("name"), "arguments": params.get("arguments", {})})
        result = call(params.get("name"), params.get("arguments") or {})
    else:
        result = {}
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}) + "\n")
    sys.stdout.flush()
