#!/usr/bin/env python3
"""Test fixture: fake `zeron mcp` stdio child. Test-authored; never used
against production data. Scenario is argv[1]."""
import json
import sys
import time

scenario = sys.argv[1] if len(sys.argv) > 1 else "normal"

CHATS = {
    "total": 2,
    "chats": [
        {
            "id": "c1",
            "title": "Fix bug",
            "status": "working",
            "statusStale": False,
            "lastMessageAt": "2025-01-01T00:00:00Z",
            "parentChatId": None,
            "deviceId": "dev1",
            "lastTurnOutcome": None,
        },
        {
            "id": "c2",
            "title": "Question",
            "status": "awaitingInput",
            "statusStale": False,
            "lastMessageAt": "2025-01-01T00:00:00Z",
            "parentChatId": "c1",
            "deviceId": "dev1",
            "lastTurnOutcome": None,
        },
    ],
}


def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def tool_result(rid, payload, structured):
    if structured:
        return {
            "jsonrpc": "2.0",
            "id": rid,
            "result": {
                "structuredContent": payload,
                "content": [],
                "isError": False,
            },
        }
    return {
        "jsonrpc": "2.0",
        "id": rid,
        "result": {
            "content": [{"type": "text", "text": json.dumps(payload)}],
            "isError": False,
        },
    }


structured = scenario != "text"

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    method = req.get("method")
    rid = req.get("id")
    if method == "notifications/initialized":
        continue
    if method == "initialize":
        send(
            {
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "serverInfo": {"name": "fake-zeron", "version": "0"},
                },
            }
        )
        continue
    if method != "tools/call":
        continue

    if scenario == "eof":
        sys.exit(0)
    if scenario == "slow":
        time.sleep(30)
        continue
    if scenario == "overlong":
        sys.stdout.write("x" * (1048576 + 2) + "\n")
        sys.stdout.flush()
        continue
    if scenario == "bad_json":
        sys.stdout.write("this is not json at all\n")
        sys.stdout.flush()
        continue
    if scenario == "wrong_id":
        send(
            {
                "jsonrpc": "2.0",
                "id": (rid or 0) + 100,
                "result": {"structuredContent": {}, "isError": False},
            }
        )
        continue
    if scenario == "rpc_error":
        send(
            {
                "jsonrpc": "2.0",
                "id": rid,
                "error": {"code": -32601, "message": "method not found"},
            }
        )
        continue
    if scenario == "mcp_error":
        send(
            {
                "jsonrpc": "2.0",
                "id": rid,
                "result": {
                    "isError": True,
                    "content": [{"type": "text", "text": "boom"}],
                },
            }
        )
        continue

    name = req.get("params", {}).get("name")
    if name == "whoami":
        send(tool_result(rid, {"localDeviceId": "dev1"}, structured))
    elif name == "list_chats":
        if scenario == "truncated":
            payload = dict(CHATS)
            payload["total"] = 900
            send(tool_result(rid, payload, structured))
        else:
            send(tool_result(rid, CHATS, structured))
