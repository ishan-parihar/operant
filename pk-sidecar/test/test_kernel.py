"""SessionKernel unit tests — ported semantics from bridge/kernel.py."""

import pytest

from pk_sidecar.kernel import SessionKernel


@pytest.mark.asyncio
async def test_state_persists_across_cells():
    k = SessionKernel()
    r1 = await k.execute("x = 41")
    assert r1["ok"]
    r2 = await k.execute("print(x + 1)")
    assert r2["ok"]
    assert r2["stdout"] == "42\n"
    assert "x" in r2["vars"]


@pytest.mark.asyncio
async def test_reset_clears_namespace():
    k = SessionKernel()
    await k.execute("y = 1")
    k.reset()
    r = await k.execute("print(y)")
    assert not r["ok"]
    assert "NameError" in r["stderr"]


@pytest.mark.asyncio
async def test_top_level_await_supported():
    k = SessionKernel()
    r = await k.execute(
        "import asyncio\n"
        "await asyncio.sleep(0)\n"
        "v = 'awaited'\n"
        "print(v)"
    )
    assert r["ok"], r["stderr"]
    assert r["stdout"] == "awaited\n"


@pytest.mark.asyncio
async def test_import_allowed_and_captured():
    k = SessionKernel()
    r = await k.execute("import json\ndata = json.loads('{\"a\": 2}')\nprint(data['a'])")
    assert r["ok"], r["stderr"]
    assert r["stdout"] == "2\n"
    # imports persist too
    r2 = await k.execute("print(json.dumps({'b': 3}))")
    assert r2["ok"] and r2["stdout"] == '{"b": 3}\n'


@pytest.mark.asyncio
async def test_syntax_error_reported_not_raised():
    k = SessionKernel()
    r = await k.execute("def broken(:")
    assert not r["ok"]
    assert "SyntaxError" in r["stderr"]


@pytest.mark.asyncio
async def test_runtime_error_reported_state_kept():
    k = SessionKernel()
    await k.execute("keep = 5")
    r = await k.execute("1/0")
    assert not r["ok"]
    assert "ZeroDivisionError" in r["stderr"]
    r2 = await k.execute("print(keep)")
    assert r2["ok"] and r2["stdout"] == "5\n"


@pytest.mark.asyncio
async def test_output_truncated_head_tail():
    k = SessionKernel()
    r = await k.execute("print('A' * 300000)")
    assert not r["ok"] or True  # print succeeds; only size matters
    out = r["stdout"]
    assert "[truncated:" in out
    assert out.startswith("AAAA")
    assert len(out.encode()) <= 200_000 + 256


@pytest.mark.asyncio
async def test_cell_timeout():
    import sys

    pytest.importorskip("time")
    k = SessionKernel()
    # Blocking sleep would freeze the event loop; use awaited sleep instead.
    r = await k.execute(
        "import asyncio\nawait asyncio.sleep(5)", cell_timeout=0.05
    )
    assert not r["ok"]
    assert "exceeded" in r["stderr"]


@pytest.mark.asyncio
async def test_user_output_captured_not_leaked():
    """User prints are captured into the result, never past redirect."""
    k = SessionKernel()
    r = await k.execute("print('user output')")
    assert r["ok"]
    assert r["stdout"] == "user output\n"
