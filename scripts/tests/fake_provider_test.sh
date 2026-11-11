#!/usr/bin/env bash
# Realtime agent-loop smoke against a credential-free local provider.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'kill "${SERVER_PID:-}" 2>/dev/null || true; rm -rf "$TMP"' EXIT
PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()
PY
)"
mkdir -p "$TMP/home/.operant"
cat > "$TMP/provider.py" <<'PY'
import json, sys
from http.server import BaseHTTPRequestHandler, HTTPServer
calls = 0
class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        global calls
        n=int(self.headers.get('content-length','0'))
        request=json.loads(self.rfile.read(n) or b'{}')
        calls += 1
        if request.get('stream'):
            self.send_response(200); self.send_header('content-type','text/event-stream'); self.send_header('connection','close'); self.end_headers()
            events=[
                {'id':'fake','object':'chat.completion.chunk','created':0,'model':'fake','choices':[{'index':0,'delta':{'role':'assistant','content':'LOCAL-'},'finish_reason':None}]},
                {'id':'fake','object':'chat.completion.chunk','created':0,'model':'fake','choices':[{'index':0,'delta':{'content':'FAKE-OK'},'finish_reason':'stop'}]},
            ]
            for event in events:
                self.wfile.write(('data: '+json.dumps(event)+'\n\n').encode()); self.wfile.flush()
            self.wfile.write(b'data: [DONE]\n\n'); self.wfile.flush()
            return
        if calls == 1:
            body={'id':'fake','object':'chat.completion','created':0,'model':'fake','choices':[{'index':0,'message':{'role':'assistant','content':None,'tool_calls':[{'id':'call_verify','type':'function','function':{'name':'verify_task','arguments':json.dumps({'action':'ad_hoc','checks':[{'cmd':'printf VERIFY-TOOL-OK'}]})}}]},'finish_reason':'tool_calls'}],'usage':{'prompt_tokens':1,'completion_tokens':1,'total_tokens':2}}
        else:
            body={'id':'fake','object':'chat.completion','created':0,'model':'fake','choices':[{'index':0,'message':{'role':'assistant','content':'LOCAL-FAKE-OK'},'finish_reason':'stop'}],'usage':{'prompt_tokens':1,'completion_tokens':1,'total_tokens':2}}
        raw=json.dumps(body).encode()
        self.send_response(200); self.send_header('content-type','application/json'); self.send_header('content-length',str(len(raw))); self.end_headers(); self.wfile.write(raw)
    def log_message(self, *_): pass
    def handle_one_request(self):
        try:
            super().handle_one_request()
        except (BrokenPipeError, ConnectionResetError):
            self.close_connection = True
HTTPServer(('127.0.0.1',int(sys.argv[1])), Handler).serve_forever()
PY
cat > "$TMP/mcp_server.py" <<'PY'
import json, sys
for line in sys.stdin:
    line=line.strip()
    if not line:
        continue
    try:
        req=json.loads(line)
    except Exception:
        continue
    rid=req.get('id')
    if rid is None:
        continue
    method=req.get('method')
    if method=='initialize':
        result={'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'fake','version':'1.0'}}
    elif method=='tools/list':
        result={'tools':[{'name':'echo','description':'Echo MCP marker','inputSchema':{'type':'object'}}]}
    elif method=='tools/call':
        result={'content':[{'type':'text','text':'MCP-STDIO-OK'}]}
    else:
        result={}
    sys.stdout.write(json.dumps({'jsonrpc':'2.0','id':rid,'result':result})+chr(10))
    sys.stdout.flush()
PY
python3 "$TMP/provider.py" "$PORT" & SERVER_PID=$!
for _ in $(seq 1 50); do
  if python3 - "$PORT" <<'PY' 2>/dev/null
import socket,sys
s=socket.create_connection(('127.0.0.1',int(sys.argv[1])),.1); s.close()
PY
  then break; fi
  sleep .05
done
cat > "$TMP/operant.toml" <<EOF
[client]
base_url = "http://127.0.0.1:$PORT/v1"
[agent]
model = "fake"
stream = false
context_window = 20000
[tui]
rich_output = false

[[mcp.servers]]
name = "fake"
transport = "stdio"
command = "python3"
args = ["$TMP/mcp_server.py"]
enabled = true
deferred = false
EOF
REAL_HOME="${HOME:?}"
# The binary may still read the real HOME for immutable configuration while
# HERMES_HOME owns writes. Snapshot operator configuration only: the running
# gateway legitimately writes transcripts and turn state under the real home,
# so hashing the whole tree would race the daemon instead of testing this run.
snapshot_real_config() {
  for path in "$REAL_HOME/.operant/operant.toml" "$REAL_HOME/.operant/.env" \
              "$REAL_HOME/.operant/settings.json" "$REAL_HOME/.operant/auth.json"; do
    if [ -f "$path" ]; then sha256sum "$path"; fi
  done
}
BEFORE="$(snapshot_real_config)"
OUT="$(HOME="$TMP/home" HERMES_HOME="$TMP/home/.operant" OPENAI_API_KEY=fake \
  "${OPERANT_BIN:-$ROOT/target/debug/operant}" -c "$TMP/operant.toml" run --query 'hello' 2>&1)"
STREAM_OUT="$(HOME="$TMP/home" HERMES_HOME="$TMP/home/.operant" OPENAI_API_KEY=fake \
  "${OPERANT_BIN:-$ROOT/target/debug/operant}" -c "$TMP/operant.toml" run --query 'hello stream' --stream 2>&1)"
MCP_OUT="$(HOME="$TMP/home" HERMES_HOME="$TMP/home/.operant" \
  "${OPERANT_BIN:-$ROOT/target/debug/operant}" -c "$TMP/operant.toml" mcp test fake 2>&1)"
AFTER="$(snapshot_real_config)"
grep -q 'LOCAL-FAKE-OK' <<<"$OUT"
grep -q 'tool_turns=1' <<<"$OUT"
grep -q 'LOCAL-FAKE-OK' <<<"$STREAM_OUT"
grep -q 'echo' <<<"$MCP_OUT"
[ "$BEFORE" = "$AFTER" ] || { echo 'real ~/.operant configuration changed' >&2; exit 1; }
echo 'fake provider realtime HTTP/SSE/tool/MCP smoke: ok'
