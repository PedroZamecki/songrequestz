# /// script
# dependencies = ["websockets"]
# ///
"""A fake Streamer.bot WebSocket server: sends Twitch chat you type and prints what songrequestz sends.

    uv run dev/sbmock.py                      # ws://127.0.0.1:8081/, no password
    uv run dev/sbmock.py --password secret    # authentication on, like Streamer.bot's setting
    uv run dev/sbmock.py --port 8080          # instead of the real Streamer.bot (close it first)

Then set songrequestz's Streamer.bot URL to ws://127.0.0.1:8081/ (songrequestz.json).
Type chat lines on stdin: `!song` (from viewer Ana), `Bob: !ssr never gonna`, `Mia(mod): !skip`.
Roles: viewer, vip, mod, broadcaster; add `+sub` for a subscriber, e.g. `Ana(vip+sub): hi`.
Like the real one, SendMessage needs authentication on.
"""

import argparse
import asyncio
import base64
import hashlib
import json
import re
import sys

import websockets

ROLES = {"viewer": 1, "vip": 2, "mod": 3, "broadcaster": 4}
SALT, CHALLENGE = "s4lt", "ch4l"
subscribed = set()


def h(s):
    return base64.b64encode(hashlib.sha256(s.encode()).digest()).decode()


def chat(line):
    m = re.match(r"(\w+)(?:\((\w+)(\+sub)?\))?:\s*(.*)", line)
    name, role, sub, text = m.groups() if m else ("Ana", "viewer", None, line)
    data = {"user": {"role": ROLES.get(role or "viewer", 1), "subscribed": bool(sub), "id": "1",
                     "login": name.lower(), "name": name, "type": "twitch"}, "text": text}
    return json.dumps({"event": {"source": "Twitch", "type": "ChatMessage"}, "data": data})


async def handler(ws, password):
    hello = {"request": "Hello", "info": {"name": "sbmock"}}
    if password:
        hello["authentication"] = {"challenge": CHALLENGE, "salt": SALT}
    await ws.send(json.dumps(hello))
    authed = not password
    async for m in ws:
        v = json.loads(m)
        req, rid = v.get("request"), v.get("id")
        if req == "Authenticate":
            authed = v.get("authentication") == h(h(password + SALT) + CHALLENGE)
            print("auth", "ok" if authed else "WRONG PASSWORD", flush=True)
            if not authed:  # what the real one does
                await ws.close(4009, "Authentication failed")
                break
            await ws.send(json.dumps({"id": rid, "status": "ok"}))
        elif password and not authed or req == "SendMessage" and not password:
            print(req, "refused: authentication required", flush=True)
            await ws.send(json.dumps({"id": rid, "status": "error", "error": "Authentication required to use this method"}))
        elif req == "Subscribe":
            subscribed.add(ws)
            print("subscribed", v.get("events"), flush=True)
            await ws.send(json.dumps({"id": rid, "events": v.get("events"), "status": "ok"}))
        elif req == "SendMessage":
            print("SendMessage", repr(v.get("message")), flush=True)
            await ws.send(json.dumps({"id": rid, "status": "ok"}))
        elif req == "DoAction":
            print("DoAction", repr(v["action"]["name"]), v.get("args", {}), flush=True)
            await ws.send(json.dumps({"id": rid, "status": "ok"}))
    subscribed.discard(ws)


async def main():
    p = argparse.ArgumentParser()
    p.add_argument("--port", type=int, default=8081)
    p.add_argument("--password", default="")
    a = p.parse_args()
    async with websockets.serve(lambda ws: handler(ws, a.password), "127.0.0.1", a.port):
        print(f"fake Streamer.bot on ws://127.0.0.1:{a.port}/" + (" (password on)" if a.password else ""), flush=True)
        loop = asyncio.get_running_loop()
        while line := await loop.run_in_executor(None, sys.stdin.readline):
            if line.strip():
                websockets.broadcast(subscribed, chat(line.strip()))


asyncio.run(main())
