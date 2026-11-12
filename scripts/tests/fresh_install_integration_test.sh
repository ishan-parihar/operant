#!/usr/bin/env bash
# scripts/tests/fresh_install_integration_test.sh — a FRESH installation must
# wire the surrounding infrastructure.
#
# Asserts, without touching the real home or the real Herdr:
#   1. `operant doctor --fix` in an empty HOME writes the Herdr plugin tree
#      and asks Herdr to link it
#   2. it registers operant in the Omarchy extra-agent registry
#   3. it writes the three user-level Omarchy wrappers
#   4. it seeds the Omarchy menu extension with the operant row, and the
#      seeded file still parses
#   5. a second run changes nothing — the install is idempotent
#
# A fake HOME and a fake `herdr` on PATH. The real Herdr is a SERVER: its
# CLI writes the link into the server's own ~/.config/herdr, which is the
# real home, so a test that called it would register the sandbox's plugin
# path on the user's machine.
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_DIR="$(cd "$SCRIPTS_DIR/.." && pwd)"
OPERANT_BIN="${OPERANT_BIN:-$REPO_DIR/target/release/operant}"

if [ ! -x "$OPERANT_BIN" ]; then
  echo "skip: no operant binary at $OPERANT_BIN (build one, or set OPERANT_BIN)" >&2
  exit 0
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

HOME_DIR="$TMP/home"
BIN_DIR="$TMP/bin"
mkdir -p "$HOME_DIR" "$BIN_DIR"

# ── a fake herdr ─────────────────────────────────────────────────────────────
cat > "$BIN_DIR/herdr" <<EOF
#!/usr/bin/env bash
echo "\$*" >> "$TMP/herdr.log"
case "\$1" in
  --version) echo "herdr 0.9.5"; exit 0 ;;
  plugin)
    [ "\$2" = "list" ] && { echo "0 plugins installed:"; exit 0; }
    exit 0
    ;;
esac
exit 0
EOF
chmod +x "$BIN_DIR/herdr"

run_fix() {
  # The Herdr environment variables are UNSET on purpose: HERDR_BIN_PATH
  # points at the real herdr inside a Herdr pane, and a developer's shell
  # could carry one, which would make the stub unreachable and the test
  # would be asserting against the real registry.
  env -u HERDR_BIN_PATH -u HERDR_ENV -u HERDR_SOCKET_PATH -u HERDR_PANE_ID \
    HOME="$HOME_DIR" PATH="$BIN_DIR:/usr/bin:/bin" "$OPERANT_BIN" doctor --fix 2>&1
}

fail() { echo "FAIL: $*" >&2; exit 1; }

# ── 1. the first run wires everything ────────────────────────────────────────
OUT="$(run_fix)" || fail "doctor --fix failed on a fresh home"
echo "$OUT" | grep -q "Done\." || fail "doctor --fix did not finish"

PLUGIN_ROOT="$HOME_DIR/.operant/integrations/herdr/operant.integration"
[ -f "$PLUGIN_ROOT/herdr-plugin.toml" ] || fail "the Herdr plugin manifest was not written"
[ -x "$PLUGIN_ROOT/scripts/launch.sh" ] || fail "the launch script was not written executable"
grep -q '^plugin link ' "$TMP/herdr.log" || fail "Herdr was never asked to link the plugin"
grep -q "plugin link $PLUGIN_ROOT" "$TMP/herdr.log" || fail "Herdr was asked to link the wrong path"

# ── 2. the Omarchy registry ──────────────────────────────────────────────────
REGISTRY="$HOME_DIR/.config/omarchy/defaults/extra-agents/operant"
[ -f "$REGISTRY" ] || fail "operant was not registered as an extra agent"
grep -q 'omarchy-operant' "$REGISTRY" || fail "the registry points at the wrong launcher"

# ── 3. the user-level wrappers ───────────────────────────────────────────────
for wrapper in omarchy-default-agent omarchy-agent omarchy-operant; do
  [ -x "$HOME_DIR/.local/bin/$wrapper" ] || fail "$wrapper was not written executable"
done

# ── 4. the menu row, and a menu that still parses ───────────────────────────
MENU="$HOME_DIR/.config/omarchy/extensions/omarchy-menu.jsonc"
[ -f "$MENU" ] || fail "the menu extension was not seeded"
grep -q 'setup.default.agent.operant' "$MENU" || fail "the menu has no operant row"

# The seeded file is JSONC: comments and a trailing comma are allowed, so it
# is parsed the way the menu shell parses it, not with a strict JSON reader.
python3 - "$MENU" <<'PY' || fail "the seeded menu does not parse"
import json, re, sys
text = open(sys.argv[1]).read()
out, i, n = [], 0, len(text)
while i < n:
    c = text[i]
    if c == '"':
        j = i + 1
        while j < n:
            if text[j] == '\\':
                j += 2
                continue
            if text[j] == '"':
                break
            j += 1
        out.append(text[i:j + 1])
        i = j + 1
        continue
    if text.startswith('//', i):
        j = text.find('\n', i)
        j = n if j < 0 else j
        out.append(' ' * (j - i))
        i = j
        continue
    if text.startswith('/*', i):
        j = text.find('*/', i)
        j = n if j < 0 else j + 2
        out.append(' ' * (j - i))
        i = j
        continue
    out.append(c)
    i += 1
body = re.sub(r',(\s*[}\]])', r'\1', ''.join(out))
menu = json.loads(body)
row = menu["setup.default.agent.operant"]
assert row["action"] == "operant-default-agent" or row["action"] == "omarchy-default-agent operant", row
assert "operant" in row["label"].lower()
PY

# ── 5. the second run changes nothing ───────────────────────────────────────
BEFORE="$(find "$HOME_DIR/.config/omarchy" "$HOME_DIR/.local/bin" "$PLUGIN_ROOT" \
  -type f -exec md5sum {} + 2>/dev/null | sort)"
run_fix >/dev/null || fail "the second doctor --fix failed"
AFTER="$(find "$HOME_DIR/.config/omarchy" "$HOME_DIR/.local/bin" "$PLUGIN_ROOT" \
  -type f -exec md5sum {} + 2>/dev/null | sort)"
[ "$BEFORE" = "$AFTER" ] || fail "the second run changed the installed files"

echo "ok: a fresh installation wires Herdr and the Omarchy agent menu, idempotently"
