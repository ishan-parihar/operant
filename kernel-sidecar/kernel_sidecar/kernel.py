"""Stateful per-session Python kernel service.

Semantics ported from hermes-prime-bridge ``bridge/kernel.py`` (persistent
globals per session_key, capped output, vars echo) and re-hosted on a
single-threaded asyncio loop so model code can ``await`` host services:
``await operant_tool(name, args_json)`` round-trips one bridged tool call to
the Rust supervisor over the NDJSON channel (Phase 2.5 tool bridge).

Cells run inside an implicit ``async def __kernel_cell__():`` wrapper, so both
sync statements and top-level ``await`` work, matching prime-agent's REPL
contract. User stdout/stderr are redirected into StringIO during execution so
they can never corrupt the NDJSON protocol channel.
"""

from __future__ import annotations

import ast
import asyncio
import io
import json
import traceback
from contextlib import redirect_stderr, redirect_stdout
from typing import Any

MAX_OUTPUT_BYTES = 200_000

# Head/tail truncation parity with operant's read_capped (head 40% + tail 60%).
_HEAD_FRACTION = 2 / 5


def _truncate(text: str, cap: int = MAX_OUTPUT_BYTES) -> tuple[str, bool]:
    if len(text.encode("utf-8", "replace")) <= cap:
        return text, False
    # Byte-accurate cap with char-boundary-safe slicing.
    head_chars = int(len(text) * _HEAD_FRACTION)
    while head_chars > 0 and len(text[:head_chars].encode("utf-8")) > cap * _HEAD_FRACTION:
        head_chars -= 1
    tail_budget = max(0, cap - len(text[:head_chars].encode("utf-8")))
    tail = ""
    for start in range(len(text), -1, -1):
        candidate = text[start:]
        if len(candidate.encode("utf-8")) <= tail_budget or start == 0:
            tail = candidate if start != 0 else ""
            break
    marker = f"\n... [truncated: output exceeded {cap} bytes] ...\n"
    return text[:head_chars] + marker + tail, True


class SessionKernel:
    """One persistent namespace per session_key (asyncio-single-threaded)."""

    def __init__(self, session_key: str = "default") -> None:
        self.session_key = session_key
        self._globals: dict[str, Any] = self._fresh_globals()
        self._counter = 0

    @staticmethod
    def _safe_builtins() -> dict[str, Any]:
        import builtins as _b

        allowed = (
            "print", "len", "range", "list", "dict", "set", "str", "int",
            "float", "bool", "sorted", "min", "max", "sum", "abs", "round",
            "isinstance", "getattr", "hasattr", "enumerate", "Exception",
            "repr", "zip", "map", "filter", "any", "all", "tuple", "bytes",
            "frozenset", "hash", "iter", "next", "ord", "chr", "divmod",
            "ValueError", "TypeError", "KeyError", "IndexError", "RuntimeError",
            "StopIteration", "ZeroDivisionError", "ArithmeticError", "OSError",
            "staticmethod", "classmethod", "property", "object", "super",
            "reversed", "slice", "format", "vars", "dir", "id", "callable",
        )
        safe = {name: getattr(_b, name) for name in allowed if hasattr(_b, name)}
        # Deliberate (upstream parity): full __import__ — this is NOT a sandbox.
        safe["__import__"] = __import__
        safe["__build_class__"] = _b.__build_class__
        return safe

    def _fresh_globals(self) -> dict[str, Any]:
        g: dict[str, Any] = {"__name__": "__kernel__"}
        g["__builtins__"] = self._safe_builtins()
        return g

    def reset(self) -> None:
        self._globals = self._fresh_globals()
        self._counter = 0

    def install(self, name: str, value: Any) -> None:
        """Inject/refresh a builtin into every future lookup (tool bridge shim)."""
        self._globals[name] = value

    def bind_skill(self, import_path: str, callable: str) -> dict[str, object]:
        """Plan 016, phase 6b: bind SKILL.toml [reference] into this session's globals.

        Supports `import_path` as dotted module (e.g. "my_pkg.sub") with
        `callable` as attribute, or single `import_path` containing a colon
        like "pkg:func" for re-exported callables. Captures ImportError as
        `_<callable>_import_error` binding so cells can inspect failure."""
        try:
            if ":" in import_path:
                mod_name, attr = import_path.split(":", 1)
                callable = attr or callable
                import_path = mod_name
            mod = __import__(import_path, fromlist=[callable])
            # Handle dotted import_path like "a.b.c" — __import__ returns top-level,
            # so walk dotted path.
            cur: object = mod
            for part in import_path.split(".")[1:]:
                cur = getattr(cur, part)
                mod = cur  # type: ignore[assignment]
            value = getattr(mod, callable)
            self._globals[callable] = value
            return {"ok": True, "bound": callable, "import": import_path}
        except BaseException as exc:  # noqa: BLE03
            err = f"{type(exc).__name__}: {exc}"
            self._globals[f"_{callable}_import_error"] = err
            return {"ok": False, "error": err, "import": import_path, "callable": callable}

    def names(self) -> list[str]:
        user = [k for k in self._globals if not k.startswith("__")]
        return sorted(user)

    async def execute(self, code: str, cell_timeout: float | None = None) -> dict[str, Any]:
        """Execute one cell; returns marshalled result (never raises)."""
        out = io.StringIO()
        err = io.StringIO()
        error: str | None = None
        try:
            wrapped = self._compile_wrapped(code)
        except SyntaxError as exc:
            # Fall back to eval-style bare-expression cells.
            try:
                wrapped = self._compile_expr(code)
            except SyntaxError:
                return {
                    "ok": False,
                    "stdout": "",
                    "stderr": "".join(traceback.format_exception_only(type(exc), exc)),
                    "error": True,
                    "vars": self.names(),
                }
        try:
            self._counter += 1
            exec(wrapped, self._globals)  # noqa: S102 - deliberate kernel semantics
            coro = self._globals["__kernel_cell__"]()
            with redirect_stdout(out), redirect_stderr(err):
                if cell_timeout:
                    await asyncio.wait_for(coro, timeout=cell_timeout)
                else:
                    await coro
        except asyncio.TimeoutError:
            error = f"cell exceeded {cell_timeout}s (host request_timeout budget)"
        except BaseException:  # noqa: BLE03 - kernel reports, never propagates
            error = traceback.format_exc()
        stdout, s_trunc = _truncate(out.getvalue())
        stderr_body = err.getvalue() or (error or "")
        stderr, e_trunc = _truncate(stderr_body)
        return {
            "ok": error is None,
            "stdout": stdout,
            "stderr": stderr,
            "error": error is not None,
            "timed_out": "exceeded" in (error or ""),
            "stdout_truncated": s_trunc,
            "stderr_truncated": e_trunc,
            "vars": self.names(),
        }

    def _compile_wrapped(self, code: str) -> Any:
        tree = ast.parse(code, mode="exec")
        # Cells run as an async function so top-level await works, but plain
        # assignment inside a function is LOCAL. Declare every assigned name
        # `global` so all state lands in the persistent session namespace
        # (REPL semantics parity with bridge/kernel.py exec-into-globals).
        names = sorted(_assigned_names(tree.body))
        indented = "\n".join(("    " + line) if line.strip() else "" for line in code.splitlines())
        header = f"async def __kernel_cell__():\n"
        if names:
            header += f"    global {', '.join(names)}\n"
        return compile(header + indented + "\n", f"<kernel:{self.session_key}>", "exec")

    def _compile_expr(self, code: str) -> Any:
        tree = ast.parse(code, mode="eval")
        src = (
            "async def __kernel_cell__():\n"
            f"    __kernel_result__ = eval({ast.literal_eval(compile(ast.Expression(tree.body), '<expr>', 'eval'))})\n"  # type: ignore[arg-type]
            "    print(__kernel_result__)\n"
        )
        return compile(src, f"<kernel:{self.session_key}>", "exec")


class KernelRegistry:
    """Per-session kernel registry (bridge/kernel.py parity)."""

    def __init__(self) -> None:
        self._kernels: dict[str, SessionKernel] = {}

    def get(self, session_key: str | None = None) -> SessionKernel:
        key = session_key or "default"
        k = self._kernels.get(key)
        if k is None:
            k = SessionKernel(key)
            self._kernels[key] = k
        return k

    def drop(self, session_key: str | None = None) -> None:
        self._kernels.pop(session_key or "default", None)

    def reset_all(self) -> None:
        for k in self._kernels.values():
            k.reset()

    def size(self) -> int:
        return len(self._kernels)


def make_operant_tool(
    session_key: str,
    call_bridge: "ToolBridge",
) -> Any:
    """Build the ``operant_tool`` async callable injected into a namespace."""

    async def operant_tool(name: str, args_json: str | dict | None = None) -> dict[str, Any]:
        if isinstance(args_json, dict):
            args = args_json
        elif args_json is None or args_json == "":
            args = {}
        elif isinstance(args_json, str):
            args = json.loads(args_json)
        else:
            raise TypeError(f"args_json must be str or dict, got {type(args_json).__name__}")
        if not isinstance(args, dict):
            raise ValueError("args must decode to an object")
        return await call_bridge(session_key, str(name), args)

    operant_tool.__name__ = "operant_tool"
    operant_tool.__doc__ = (
        "Call one allowlisted operant tool from inside the kernel and await its "
        "JSON result. Subject to the same approval policy as direct calls; "
        "errors are returned as values."
    )
    return operant_tool


# Type alias for readability only.
ToolBridge = Any  # async (session_key, name, args: dict) -> dict


def _assigned_names(nodes: list[ast.stmt]) -> set[str]:
    """Every name bound anywhere in the cell (top-level REPL binding rules).

    Covers assignment/annotation/augassign/walrus/for/with/function/class/
    import/delete/match-capture bindings. Names the user declared nonlocal are
    left alone (rare in model-authored cells).
    """
    out: set[str] = set()

    def target_names(t: ast.AST) -> None:
        if isinstance(t, ast.Name):
            out.add(t.id)
        elif isinstance(t, (ast.Tuple, ast.List)):
            for elt in t.elts:
                target_names(elt)
        elif isinstance(t, ast.Starred):
            target_names(t.value)
        elif isinstance(t, ast.Attribute):
            pass  # obj.attr binds on obj, not a bare name

    for node in ast.walk(ast.Module(body=nodes, type_ignores=[])):
        if isinstance(node, ast.Assign):
            for t in node.targets:
                target_names(t)
        elif isinstance(node, ast.AnnAssign):
            target_names(node.target)
        elif isinstance(node, (ast.AugAssign, ast.Delete)):
            target_names(node.target)
        elif isinstance(node, ast.NamedExpr):
            target_names(node.target)
        elif isinstance(node, (ast.For, ast.AsyncFor)):
            target_names(node.target)
        elif isinstance(node, (ast.With, ast.AsyncWith)):
            for item in node.items:
                if item.optional_vars is not None:
                    target_names(item.optional_vars)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            out.add(node.name)
        elif isinstance(node, ast.Import):
            for alias in node.names:
                out.add(alias.asname or alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            for alias in node.names:
                out.add(alias.asname or alias.name)
        elif isinstance(node, ast.Match):
            for child in ast.walk(node):
                if isinstance(child, ast.MatchAs) and child.name:
                    out.add(child.name)
                elif isinstance(child, ast.MatchStar) and child.name:
                    out.add(child.name)
                elif isinstance(child, ast.MatchMapping) and child.rest:
                    out.add(child.rest)
    return out
