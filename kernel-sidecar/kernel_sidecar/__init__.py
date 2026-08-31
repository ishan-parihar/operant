"""kernel-sidecar: persistent stateful Python kernel + continual-harness service.

NDJSON JSON-RPC over stdio, supervised by operant's Rust PkSidecarSupervisor.
The continual-harness store imports prime-agent's real ``rlm`` package from the
pinned ``vendor/prime-agent`` submodule (live upstream, never copied).
"""

from .kernel import SessionKernel, KernelRegistry
from . import vendor

__all__ = ["SessionKernel", "KernelRegistry", "vendor"]
