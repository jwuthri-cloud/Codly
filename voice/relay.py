"""Codly voice relay: talk to one Codly agent through OpenAI Realtime.

Runs next to `codly serve` on the same machine (the devbox). `codly serve` proxies its
`/api/voice/ws` to this relay's `/ws`, so the browser needs no extra tunnel, and only Codly's own
authenticated page reaches it. Auth to OpenAI is workload identity federation (GCP identity token ->
OpenAI); no API key exists, and no token reaches the browser.

The realtime model is the voice; the Codly agent is the brain. The model drives the agent through
tools, which the browser runs (it holds Codly's API and the agent's live state):
  ask_agent(message)  sends a message to the agent; its answer arrives later as `agent_reply`
  agent_status()      the agent's status and its latest message

WebSocket /ws
  Browser -> relay: binary = PCM16 mono 24 kHz mic audio; text = JSON:
    {"type": "context", "agent": name, "role": role, "provider": cli}  first frame, sets the instructions
    {"type": "tool_result", "id", "output"}                            answer to a tool_call
    {"type": "agent_reply", "text"}                                    the agent finished a turn
    {"type": "text", "text"}                                           typed message
    {"type": "interrupt"}                                              stop the current answer
  Relay -> browser: binary = PCM16 mono 24 kHz reply audio; text = JSON:
    ready, clear, user_start, user, thinking, assistant_delta, assistant_done, response_done,
    tool_call {id, name, args}, error.
"""
import asyncio
import base64
import json
import logging
import os

import httpx
import uvicorn
from fastapi import FastAPI, WebSocket, WebSocketDisconnect
from openai import AsyncOpenAI

MODEL = os.environ.get("REALTIME_MODEL", "gpt-realtime-2.1")
VOICE = os.environ.get("REALTIME_VOICE", "marin")
TRANSCRIBE_MODEL = os.environ.get("REALTIME_TRANSCRIBE_MODEL", "gpt-4o-mini-transcribe")
WIF_AUDIENCE = "https://api.openai.com"
METADATA_IDENTITY_URL = "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/identity"
BROWSER_TOOLS = {"ask_agent", "agent_status"}

TOOLS = [
    {
        "type": "function",
        "name": "ask_agent",
        "description": (
            "Send a message to the coding agent: a question, a task, a change of plan or an answer to its "
            "question. Returns at once; the agent's reply arrives later as a system message."
        ),
        "parameters": {
            "type": "object",
            "properties": {"message": {"type": "string", "description": "What to tell the agent, in full sentences."}},
            "required": ["message"],
        },
    },
    {
        "type": "function",
        "name": "agent_status",
        "description": "The agent's current status (idle, working, waiting for approval...) and its latest message.",
        "parameters": {"type": "object", "properties": {}},
    },
]

log = logging.getLogger("codly-voice")
app = FastAPI()


def instructions(agent: str, role: str, provider: str) -> str:
    return f"""You are the voice of {agent}, a Codly coding agent ({role}, running on {provider}).
The user talks to you out loud to work with {agent}. You do not write code or read files yourself:
{agent} does the work. Your job is to pass the user's intent to {agent} with ask_agent, and to tell
the user what {agent} answered.

Rules:
- When the user asks for work, a question about the code, or a decision, call ask_agent with a clear,
  complete message. Then say in a few words that {agent} is on it. Do not invent its answer.
- When a system message brings {agent}'s reply, give the gist in one to three spoken sentences. Do not
  read code, paths, diffs or long lists aloud: the user sees them on screen.
- Use agent_status when the user asks what {agent} is doing.
- Short answers, plain words, like a colleague on a call. If you do not know, say so."""


def gcp_identity_token() -> str:
    """Identity token of this Compute Engine instance, written for OpenAI. Stays in memory."""
    with httpx.Client(trust_env=False, timeout=10.0) as metadata:
        answer = metadata.get(
            METADATA_IDENTITY_URL,
            params={"audience": WIF_AUDIENCE, "format": "full"},
            headers={"Metadata-Flavor": "Google"},
        )
    if answer.status_code != 200 or answer.headers.get("Metadata-Flavor") != "Google":
        raise RuntimeError(f"metadata server gave no identity token (status {answer.status_code})")
    return answer.text.strip()


def make_client() -> AsyncOpenAI:
    """OpenAI client that signs in with workload identity federation. No API key is used."""
    return AsyncOpenAI(
        workload_identity={
            "identity_provider_id": os.environ["OPENAI_IDENTITY_PROVIDER_ID"],
            "service_account_id": os.environ["OPENAI_SERVICE_ACCOUNT_ID"],
            "provider": {"token_type": "jwt", "get_token": gcp_identity_token},
        },
    )


class Session:
    """One browser <-> one Realtime session."""

    def __init__(self, ws: WebSocket, oai):
        self.ws = ws
        self.oai = oai
        self.responding = False  # a response is in progress: a new response.create would be refused
        self.pending_reply = False  # an agent reply arrived during a response: answer it after

    async def respond(self) -> None:
        if self.responding:
            self.pending_reply = True
        else:
            self.responding = True
            await self.oai.response.create()

    async def say_as_system(self, text: str) -> None:
        await self.oai.conversation.item.create(
            item={"type": "message", "role": "system", "content": [{"type": "input_text", "text": text}]}
        )
        await self.respond()

    async def from_browser(self) -> None:
        while True:
            msg = await self.ws.receive()
            if msg["type"] == "websocket.disconnect":
                return
            if msg.get("bytes"):
                await self.oai.input_audio_buffer.append(audio=base64.b64encode(msg["bytes"]).decode())
                continue
            if not msg.get("text"):
                continue
            cmd = json.loads(msg["text"])
            kind = cmd.get("type")
            if kind == "context":
                await self.oai.session.update(
                    session={
                        "type": "realtime",
                        "instructions": instructions(cmd.get("agent", "the agent"), cmd.get("role", "agent"), cmd.get("provider", "a CLI")),
                    }
                )
            elif kind == "tool_result":
                await self.oai.conversation.item.create(
                    item={"type": "function_call_output", "call_id": cmd["id"], "output": json.dumps(cmd.get("output"))}
                )
                await self.respond()
            elif kind == "agent_reply":
                await self.say_as_system(f"The agent finished its turn. Its reply:\n\n{cmd.get('text', '')}")
            elif kind == "text" and cmd.get("text", "").strip():
                await self.oai.conversation.item.create(
                    item={"type": "message", "role": "user", "content": [{"type": "input_text", "text": cmd["text"]}]}
                )
                await self.respond()
            elif kind == "interrupt":
                if self.responding:
                    await self.oai.response.cancel()
                await self.ws.send_json({"type": "clear"})

    async def from_openai(self) -> None:
        async for ev in self.oai:
            t = ev.type
            if t == "response.output_audio.delta":
                await self.ws.send_bytes(base64.b64decode(ev.delta))
            elif t == "response.created":
                self.responding = True
            elif t == "input_audio_buffer.speech_started":
                await self.ws.send_json({"type": "clear"})  # barge-in: drop queued playback
                await self.ws.send_json({"type": "user_start"})
            elif t == "input_audio_buffer.speech_stopped":
                await self.ws.send_json({"type": "thinking"})
            elif t == "conversation.item.input_audio_transcription.completed":
                await self.ws.send_json({"type": "user", "text": ev.transcript})
            elif t == "response.output_audio_transcript.delta":
                await self.ws.send_json({"type": "assistant_delta", "text": ev.delta})
            elif t == "response.output_audio_transcript.done":
                await self.ws.send_json({"type": "assistant_done", "text": ev.transcript})
            elif t == "response.function_call_arguments.done":
                args = json.loads(ev.arguments or "{}")
                if ev.name in BROWSER_TOOLS:
                    await self.ws.send_json({"type": "tool_call", "id": ev.call_id, "name": ev.name, "args": args})
                else:  # the model named a tool that does not exist: tell it, do not hang
                    await self.oai.conversation.item.create(
                        item={"type": "function_call_output", "call_id": ev.call_id, "output": json.dumps({"error": f"unknown tool {ev.name}"})}
                    )
                    await self.respond()
            elif t == "response.done":
                self.responding = False
                await self.ws.send_json({"type": "response_done"})
                # Tool outputs and agent replies that came in during this response.
                if self.pending_reply:
                    self.pending_reply = False
                    await self.respond()
            elif t == "error":
                log.error("openai error: %s", ev.error.message)
                await self.ws.send_json({"type": "error", "text": ev.error.message})


@app.websocket("/ws")
async def ws_endpoint(ws: WebSocket) -> None:
    await ws.accept()
    try:
        # The SDK does not accept workload identity on WebSocket connections, so WIF (over REST) mints a
        # short-lived Realtime client secret, used only here to open the socket.
        secret = await make_client().realtime.client_secrets.create(session={"type": "realtime", "model": MODEL})
    except Exception as exc:  # report the failure to the page, do not hide it
        log.exception("could not mint a Realtime client secret")
        await ws.send_json({"type": "error", "text": f"OpenAI sign-in (WIF) failed: {exc}"})
        await ws.close()
        return
    async with AsyncOpenAI(api_key=secret.value).realtime.connect(model=MODEL) as oai:
        await oai.session.update(
            session={
                "type": "realtime",
                "instructions": instructions("the agent", "agent", "a CLI"),
                "tools": TOOLS,
                "tool_choice": "auto",
                "audio": {
                    "input": {
                        "format": {"type": "audio/pcm", "rate": 24000},
                        "transcription": {"model": TRANSCRIBE_MODEL},
                        "turn_detection": {"type": "semantic_vad"},
                    },
                    "output": {"format": {"type": "audio/pcm", "rate": 24000}, "voice": VOICE},
                },
            }
        )
        await ws.send_json({"type": "ready", "model": MODEL, "voice": VOICE})
        session = Session(ws, oai)
        tasks = [asyncio.create_task(session.from_browser()), asyncio.create_task(session.from_openai())]
        try:
            done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
            for task in done:
                if task.exception() and not isinstance(task.exception(), WebSocketDisconnect):
                    log.error("voice session ended: %r", task.exception())
        finally:
            for task in tasks:
                task.cancel()


if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    # Loopback only: `codly serve` on this machine is the only client.
    uvicorn.run(app, host="127.0.0.1", port=int(os.environ.get("CODLY_VOICE_PORT", "8766")))
