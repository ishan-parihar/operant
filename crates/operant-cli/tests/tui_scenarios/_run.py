#!/usr/bin/env python3
"""Phase 0 runner for the operant TUI headless visual-regression corpus.

One command, three modes:

  exec     (default) Execute every `live` scenario and every variant against the
           real binary using ONLY flags that exist today, and report PASS/FAIL.
           Assertion-only: no goldens, no filesystem state, useful while
           authoring scenarios.

  prove    The determinism proof. Render every scenario/variant TWICE in
           separate processes, against separate temp output dirs and separate
           temp HERMES_HOME fixtures, and byte-diff BOTH the text dump and the
           style dump. A scenario whose two runs disagree is NONDETERMINISTIC:
           no golden is committed for it and it is reported with the precise
           varying region. For every scenario that survives, a third render
           writes the goldens and a fourth gates the committed file.

  verify   The CI gate. One render per scenario, diffed against the committed
           text and style goldens. Exits non-zero on any drift, and also on a
           MISSING golden (the tool would silently bootstrap one, which would
           make the gate a no-op). Scenarios marked `deterministic: false` in
           their scenario file still run their assertions but are excluded
           from the golden comparison, with the reason printed.

Everything runs in its own process, sequentially: the mock path and
slash-command handling touch process-wide state (slash-usage store, notification
and background-task registries, audio detection), so two scenarios may never
share a process.

Usage:
  python3 _run.py [exec|prove|verify] [scenario-name ...]

Environment:
  OPERANT_BIN   binary under test. Defaults to <repo>/target/debug/operant,
                then ~/.cargo/bin/operant. MUST be built from the same tree as
                the corpus: a stale binary silently tests the wrong renderer.
"""
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).parent
BASELINES = ROOT / "baselines"
REPO = ROOT.parents[3]

# --- determinism pins ---------------------------------------------------------
#
# Every one of these exists because some surface reads it. Dropping a pin is
# how a golden starts failing on someone else's machine, which is how a gate
# dies.
#
#   OPERANT_COLOR_DEPTH  tui/color_depth.rs::detect() quantizes the whole
#                        palette to the detected depth, so the STYLE dump of
#                        every surface depends on COLORTERM/TERM/NO_COLOR.
#                        Unpinned, the same commit renders different fg/bg on a
#                        truecolor laptop and a 256-colour CI runner.
#                        NOTE: the vendored jcode style layer ran a SECOND,
#                        independent depth detection that never read this var,
#                        so one frame could carry both encodings — a truecolor
#                        palette plus a single `indexed:N` run wherever the
#                        per-frame buffer pass rewrote a named colour. That is
#                        fixed at the source (color_depth::align_vendored_
#                        detector), not pinned away here, so this var is now
#                        genuinely authoritative.
#   ANTHROPIC_API_KEY  TuiApp::enter auto-opens the "Connect a provider" dialog
#                        when `!app.has_credentials && !has_completed_onboarding`
#                        (tui/adapter_types/tui_app.rs:378). Both halves are
#                        machine-local: the key from the env, the flag from
#                        $HOME/.operant/settings.json. Unpinned, a developer who
#                        has onboarded and a fresh CI runner render DIFFERENT
#                        first frames, and every scenario in the corpus fails
#                        on the machine that has never run `operant setup`.
#                        A non-empty placeholder satisfies the check; the
#                        endpoint below is unroutable, so it is never used.
#   NO_COLOR removed     detect() gives it top priority: set, and every colour
#                        collapses to the same value.
#   TERM                 read by image_render/mermaid and the vendored
#                        color_support for graphics + palette decisions.
#   TZ / LANG / LC_ALL   any surface that formats a date, a time or a number.
#   PATH                 FIXED, never inherited from the developer. This is the
#                        one pin that carries real teeth, and it exists for a
#                        measured reason: operant_core::voice::detect_audio_
#                        environment() sets `available = warnings.is_empty()`
#                        and warns when none of termux-microphone-record /
#                        ffmpeg / arecord / sox resolves on PATH. TuiApp::enter
#                        feeds that to voice_mode_notice.show_if_available, so
#                        a machine WITH a recorder shows a 2-row notice from
#                        frame 0 and a machine WITHOUT one does not — which
#                        shifts the whole layout and invalidates nearly every
#                        text baseline. Measured: with PATH=/nonexistent,
#                        help-overlay / footer-bar / diff-viewer-git /
#                        global-search all change text, and voice-mode-notice
#                        exits 1 because its assertion requires the notice.
#                        A declared fixed value stops the corpus silently
#                        inheriting a developer's shell PATH.
#                        RESIDUAL, not pinnable from here: /.dockerenv and
#                        /proc/version are file facts, not env vars, and each
#                        forces `available = false` on a Docker/WSL runner. The
#                        corpus is therefore reproducible on a non-container,
#                        non-WSL, non-SSH host whose PATH resolves a recorder —
#                        NOT on literally any machine. Closing that needs a
#                        product change (gate the notice on config, not on
#                        hardware probing), not another pin.
#   HOME + HERMES_HOME   skills, plugins, sessions, auth, slash-usage.json and
#                        every other machine-local read. Pointed at a fresh
#                        empty fixture per run so a developer's 105 installed
#                        skills can never leak into a golden. An EMPTY fixture
#                        is the strongest pin available: a missing
#                        settings.json deserialises to Settings::default(),
#                        which is the same on every machine.
PINNED_ENV = {
    "OPERANT_COLOR_DEPTH": "truecolor",
    "TERM": "xterm-256color",
    "TZ": "UTC",
    "LANG": "C.UTF-8",
    "LC_ALL": "C.UTF-8",
    "ANTHROPIC_API_KEY": "offline-capture-not-a-real-key",
    # Fixed, not inherited: see the PATH note above for why this is the pin
    # that decides whether the corpus is reproducible at all.
    "PATH": "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
}

# Offline, deterministic agent config. The endpoint is unroutable on purpose:
# a scenario that reaches the network is nondeterministic BY CONSTRUCTION and
# must be excluded, not retried.
PINNED_CONFIG = """[client]
base_url = "http://127.0.0.1:9/v1"

[agent]
model = "offline-capture"
"""


def binary():
    env = os.environ.get("OPERANT_BIN")
    if env:
        return env
    for cand in (REPO / "target/debug/operant", pathlib.Path.home() / ".cargo/bin/operant"):
        if cand.exists():
            return str(cand)
    sys.exit("no operant binary found; build it or set OPERANT_BIN")


BIN = binary()


class Fixture:
    """A throwaway HOME + config for one render.

    Fresh per run on purpose: reusing one across two runs would let state
    written by run 1 (slash usage, session rows) show up in run 2 as a diff
    that is really a harness bug.
    """

    def __init__(self, tmpdir):
        self.dir = pathlib.Path(tmpdir)
        self.home = self.dir / "home"
        (self.home / ".operant").mkdir(parents=True, exist_ok=True)
        self.config = self.dir / "operant.toml"
        self.config.write_text(PINNED_CONFIG)

    def env(self, extra=None):
        env = dict(PINNED_ENV)
        env["HOME"] = str(self.home)
        env["HERMES_HOME"] = str(self.home / ".operant")
        env.pop("NO_COLOR", None)
        if extra:
            env.update({k: str(v) for k, v in extra.items()})
        return env


def load_scenarios():
    out = []
    for f in sorted(ROOT.glob("*.scenario.json")):
        d = json.loads(f.read_text())
        d["_file"] = f
        out.append(d)
    return out


def runs_of(d):
    """Every render the corpus defines: the scenario itself, then each variant.

    Yields a dict per run: label, the two committed baseline filenames, and the
    effective keys/size/assert/assert_screen. A variant inherits the scenario's
    keys/assert/assert_screen unless it overrides them — the schema's
    inheritance rule.
    """
    yield {
        "label": d["name"],
        "text": d["baseline"],
        "style": d["style_baseline"],
        "keys": d["keys"],
        "size": d["size"],
        "assert": d["assert"],
        "assert_screen": d.get("assert_screen"),
    }
    for v in d.get("variants") or []:
        size = v["size"]
        stem = pathlib.Path(d["baseline"]).stem  # "<name>"
        style_stem = pathlib.Path(d["style_baseline"]).stem  # "<name>.style"
        yield {
            "label": f"{d['name']}.{size}",
            # schema `variants`: "<baseline stem>.<W>x<H>.txt" / ".style.txt"
            "text": f"{stem}.{size}.txt",
            "style": f"{style_stem}.{size}.txt",
            "keys": v.get("keys", d["keys"]),
            "size": size,
            "assert": v.get("assert", d["assert"]),
            "assert_screen": v.get("assert_screen", d.get("assert_screen")),
        }


def command(d, run):
    cmd = [BIN, "-c", "@CONFIG@", "tui", "debug", "simulate",
           "--keys", run["keys"], "--size", run["size"]]
    cmd += d.get("flags") or []
    if run["assert"]:
        cmd += ["--assert", ",".join(run["assert"])]
    if run["assert_screen"]:
        cmd += ["--assert-screen", ",".join(run["assert_screen"])]
    if d.get("agent_script"):
        cmd += ["--agent-script", str(ROOT / "agent_scripts" / d["agent_script"])]
    return cmd


def render(d, run, *, dump_text=None, dump_style=None,
           gate_text=None, gate_style=None, accept=False, fixture=None):
    """One render in its own process. Returns (returncode, combined output)."""
    tmp = fixture or Fixture(tempfile.mkdtemp(prefix="operant-tui-"))
    cmd = [c.replace("@CONFIG@", str(tmp.config)) for c in command(d, run)]
    if dump_text:
        cmd += ["--dump-screen", str(dump_text)]
    if dump_style:
        cmd += ["--dump-style", str(dump_style)]
    if gate_text:
        cmd += ["--baseline", str(gate_text)]
    if gate_style:
        cmd += ["--style-baseline", str(gate_style)]
    if accept:
        cmd += ["--accept-baseline"]
    env = tmp.env(d.get("env"))
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=180, env=env)
        return p.returncode, (p.stdout + p.stderr)
    finally:
        if fixture is None:
            shutil.rmtree(tmp.dir, ignore_errors=True)


def failure_detail(out):
    errs = [l for l in out.splitlines() if l.startswith("Error:")]
    if errs:
        return errs[-1]
    tail = [l for l in out.splitlines() if l.strip()]
    return tail[-1] if tail else "(no output)"


def text_diff_region(a, b):
    """First differing (row, col) between two text dumps, human-readable.

    Only used to explain WHY a scenario is nondeterministic; the gate itself
    never masks anything.
    """
    la, lb = a.splitlines(), b.splitlines()
    for i in range(max(len(la), len(lb))):
        ra = la[i] if i < len(la) else "<missing row>"
        rb = lb[i] if i < len(lb) else "<missing row>"
        if ra != rb:
            col = next((c for c in range(min(len(ra), len(rb))) if ra[c] != rb[c]),
                       min(len(ra), len(rb)))
            return f"row {i}, col {col}: {ra!r} != {rb!r}"
    return "identical"


def style_diff_region(a, b):
    la, lb = a.splitlines(), b.splitlines()
    if not la or not lb or la[0] != lb[0]:
        return f"header: {la[:1]} != {lb[:1]}"
    for i in range(1, max(len(la), len(lb))):
        ra = la[i].split(" ") if i < len(la) else []
        rb = lb[i].split(" ") if i < len(lb) else []
        if len(ra) != len(rb):
            return f"row {i - 1}: {len(ra)} vs {len(rb)} tokens"
        for c, (x, y) in enumerate(zip(ra, rb)):
            if x != y:
                return f"row {i - 1}, col {c}: {x} != {y}"
    return "identical"


def excluded_count():
    p = ROOT / "excluded.json"
    return len(json.loads(p.read_text())["excluded"]) if p.exists() else 0


# ---------------------------------------------------------------------------
# modes
# ---------------------------------------------------------------------------

def mode_exec(scenarios, only):
    rows, fails = [], []
    for d in scenarios:
        if d.get("reachability") == "blocked":
            continue
        if only and d["name"] not in only:
            continue
        for run in runs_of(d):
            rc, out = render(d, run)
            status = "PASS" if rc == 0 else "FAIL"
            detail = "" if rc == 0 else failure_detail(out)
            if rc != 0:
                fails.append((run["label"], detail))
            rows.append((status, run["label"], detail))
    w = max(len(r[0]) for r in rows) if rows else 10
    for status, name, detail in rows:
        print(f"{status}  {name:<{w}}" + (f"   {detail}" if detail else ""))
    print()
    print(f"total scenarios executed     : {len(rows)}")
    print(f"surfaces parked in excluded  : {excluded_count()}")
    print(f"pass                           : {sum(1 for r in rows if r[0] == 'PASS')}")
    print(f"fail                           : {sum(1 for r in rows if r[0] == 'FAIL')}")
    if fails:
        print("\nfailures:")
        for n, det in fails:
            print(f"  {n}: {det}")
        return 1
    return 0


def mode_prove(scenarios, only):
    """Double-render, byte-diff, then write + re-verify with a third render."""
    BASELINES.mkdir(exist_ok=True)
    proven, unstable, nondet, broken = [], [], [], []
    for d in scenarios:
        if d.get("reachability") == "blocked":
            continue
        if only and d["name"] not in only:
            continue
        for run in runs_of(d):
            label, text_name, style_name = (run["label"], run["text"], run["style"])
            if d.get("deterministic") is False:
                print(f"SKIP  {label}  declared nondeterministic: "
                      f"{d.get('nondeterminism', {}).get('reason', '(no reason)')}")
                nondet.append(label)
                continue
            work = pathlib.Path(tempfile.mkdtemp(prefix="operant-tui-prove-"))
            try:
                a_txt, a_sty = work / "a.txt", work / "a.style.txt"
                b_txt, b_sty = work / "b.txt", work / "b.style.txt"
                rc1, o1 = render(d, run, dump_text=a_txt, dump_style=a_sty)
                rc2, o2 = render(d, run, dump_text=b_txt, dump_style=b_sty)
                if rc1 != 0 or rc2 != 0:
                    broken.append((label, failure_detail(o1 if rc1 else o2)))
                    print(f"BROKEN {label}  {failure_detail(o1 if rc1 else o2)}")
                    continue
                t_a, t_b = a_txt.read_text(), b_txt.read_text()
                s_a, s_b = a_sty.read_text(), b_sty.read_text()
                if t_a != t_b or s_a != s_b:
                    why = []
                    if t_a != t_b:
                        why.append("text " + text_diff_region(t_a, t_b))
                    if s_a != s_b:
                        why.append("style " + style_diff_region(s_a, s_b))
                    nondet.append(label)
                    print(f"NONDET {label}  {'; '.join(why)}")
                    continue
                # Third and fourth renders. C writes the goldens through the
                # tool's own writer, so the committed format is exactly what
                # the gate compares; D gates that committed file with no accept
                # flag, which is byte-for-byte the check CI runs. A D that
                # fails means two agreeing renders were luck, not determinism.
                gold_txt, gold_sty = BASELINES / text_name, BASELINES / style_name
                rc3, o3 = render(d, run, gate_text=gold_txt, gate_style=gold_sty, accept=True)
                if rc3 != 0:
                    broken.append((label, failure_detail(o3)))
                    print(f"BROKEN {label}  {failure_detail(o3)}")
                    continue
                rc4, o4 = render(d, run, gate_text=gold_txt, gate_style=gold_sty)
                if rc4 != 0:
                    gold_txt.unlink(missing_ok=True)
                    gold_sty.unlink(missing_ok=True)
                    unstable.append(label)
                    print(f"UNSTABLE {label}  fourth render disagrees with the golden just "
                          f"written: {failure_detail(o4)}")
                    continue
                proven.append((label, text_name, style_name))
                print(f"PROVEN {label}  -> baselines/{text_name}, baselines/{style_name}")
            finally:
                shutil.rmtree(work, ignore_errors=True)
    print()
    print(f"deterministic + goldened : {len(proven)}")
    print(f"unstable (3rd run drift) : {len(unstable)}")
    print(f"nondeterministic         : {len(nondet)}")
    print(f"broken (non-zero exit)   : {len(broken)}")
    print(f"surfaces parked excluded : {excluded_count()}")
    if unstable or broken:
        print("\nproblems:")
        for n in unstable:
            print(f"  unstable: {n}")
        for n, det in broken:
            print(f"  broken: {n}: {det}")
        return 1
    return 0


def mode_verify(scenarios, only):
    """The CI gate. One render, diffed against both committed goldens.

    Golden existence is checked BEFORE the render: the tool bootstraps a
    missing baseline and exits 0, which would turn the gate into a no-op the
    first time anyone deleted a file. Checking first also lets a single render
    carry the assertions and both gates together.
    """
    gated, fails, skipped, missing = [], [], [], []
    for d in scenarios:
        if d.get("reachability") == "blocked":
            continue
        if only and d["name"] not in only:
            continue
        for run in runs_of(d):
            label, text_name, style_name = (run["label"], run["text"], run["style"])
            if d.get("deterministic") is False:
                # The surface is still exercised: its assertions still run, it
                # just has no golden to diff against.
                rc, out = render(d, run)
                if rc != 0:
                    fails.append((label, failure_detail(out)))
                else:
                    skipped.append((label, d.get("nondeterminism", {}).get("reason", "")))
                continue
            gold_txt, gold_sty = BASELINES / text_name, BASELINES / style_name
            absent = [
                str(p.relative_to(ROOT)) for p in (gold_txt, gold_sty) if not p.exists()
            ]
            if absent:
                missing.append((label, absent))
                continue
            rc, out = render(d, run, gate_text=gold_txt, gate_style=gold_sty)
            if rc != 0:
                fails.append((label, failure_detail(out)))
            else:
                gated.append(label)
    print(f"scenarios gated on both baselines : {len(gated)}")
    if skipped:
        print(f"assertions-only (nondeterministic) : {len(skipped)}")
        for label, reason in skipped:
            print(f"    {label}: {reason}")
    if missing or fails:
        print()
        for label, paths in missing:
            print(f"MISSING GOLDEN  {label}: {', '.join(paths)}")
        for label, det in fails:
            print(f"DRIFT           {label}: {det}")
        print(
            f"\nverify FAILED: {len(missing)} missing golden(s), {len(fails)} drift(s)"
        )
        return 1
    print("verify PASSED: 0 drift")
    return 0



MODES = {"exec": mode_exec, "prove": mode_prove, "verify": mode_verify}


def main():
    argv = sys.argv[1:]
    mode = argv[0] if argv and argv[0] in MODES else "exec"
    only = argv[1:] if argv and argv[0] in MODES else argv
    return MODES[mode](load_scenarios(), set(only))


if __name__ == "__main__":
    sys.exit(main())
