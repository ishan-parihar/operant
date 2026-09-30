#!/usr/bin/env python3
"""Execute every scenario against the real binary, plus every variant, and
report PASS/FAIL per run. Also reports the count of surfaces parked in
excluded.json so a run can never quietly shrink its own net.

Flags used per scenario: --keys --size --assert --assert-screen
--agent-script, plus the scenario's own `flags` array verbatim.
Authoring tool, not part of the contract."""
import json, pathlib, subprocess, sys, tempfile, os

ROOT = pathlib.Path(__file__).parent
BIN = os.environ.get("OPERANT_BIN", str(pathlib.Path.home() / ".cargo/bin/operant"))
ONLY = sys.argv[1:]

rows, fails = [], []
for f in sorted(ROOT.glob("*.scenario.json")):
    d = json.loads(f.read_text())
    name = d["name"]
    if d.get("reachability") == "blocked":
        continue
    if ONLY and name not in ONLY:
        continue

    def run(keys, asserts, ascreen, script, flags):
        cmd = [BIN, "tui", "debug", "simulate", "--keys", keys, "--size", d["size"]]
        cmd += flags
        if asserts:
            cmd += ["--assert", ",".join(asserts)]
        if ascreen:
            cmd += ["--assert-screen", ",".join(ascreen)]
        if script:
            cmd += ["--agent-script", str(ROOT / "agent_scripts" / script)]
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=180)
        return p.returncode, (p.stdout + p.stderr).strip().splitlines()

    flags = d.get("flags") or []
    rc, out = run(d["keys"], d["assert"], d.get("assert_screen"), d.get("agent_script"), flags)
    status = "PASS" if rc == 0 else "FAIL"
    detail = ""
    if rc != 0:
        errs = [l for l in out if l.startswith("Error:")]
        detail = errs[0] if errs else out[-1] if out else "(no output)"
        fails.append((name, detail))

    # variants
    vstat = []
    for v in d.get("variants", []) or []:
        vrc, vout = run(v.get("keys", d["keys"]), v.get("assert", d["assert"]),
                        v.get("assert_screen", d.get("assert_screen")),
                        d.get("agent_script"), flags)
        vstat.append(f"{v['size']}={'ok' if vrc == 0 else 'FAIL'}")
        if vrc != 0:
            errs = [l for l in vout if l.startswith("Error:")]
            fails.append((f"{name}@{v['size']}",
                          errs[0] if errs else (vout[-1] if vout else "(no output)")))

    rows.append((status, name, ",".join(vstat), detail))

w = max(len(r[1]) for r in rows)
for status, name, vstat, detail in rows:
    print(f"{status}  {name:<{w}}  variants[{vstat}]" + (f"   {detail}" if detail else ""))

print()
excl = ROOT / "excluded.json"
n_excl = len(json.loads(excl.read_text())["excluded"]) if excl.exists() else 0
print(f"total scenarios executed     : {len(rows)}")
print(f"surfaces parked in excluded  : {n_excl}")
print(f"pass                           : {sum(1 for r in rows if r[0] == 'PASS')}")
print(f"fail                           : {sum(1 for r in rows if r[0] == 'FAIL')}")
if fails:
    print("\nfailures:")
    for n, d_ in fails:
        print(f"  {n}: {d_}")
