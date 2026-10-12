---
name: inspecting-operant-dashboard-dom
description: "Read the live Operant web dashboard DOM/CSS over CDP."
version: 1.0.0
author: Operant
license: MIT
platforms: [linux, macos, windows]
metadata:
  operant:
    tags: [dashboard, web, cdp, dom, ui-verification, self-inspection]
    related_skills: [node-inspect-debugger, systematic-debugging, dogfood]
---

# Inspecting the live Operant web dashboard DOM

## Overview

When you are working on the Operant web dashboard and the user is running it
(`operant dashboard server`), you can read the **live rendered DOM** of the page
they are looking at — computed styles, geometry, which CSS rule actually won,
console output — instead of inferring it from the source and being wrong.

The dashboard server binds `127.0.0.1:9119` by default (`--port` / `--host`
override). It serves the embedded web UI with a bearer-token API: a token is
generated at startup, printed to the console, and injected into the page as a
global so the frontend can call the API. The renderer is a Chromium page
driven through the shared Obscura CDP browser, so everything DevTools can read,
a script can read.

**This does not replace looking at it.** CDP answers *factual* questions ("what
is the computed padding", "did this element render", "which selector matches").
It cannot tell you whether the result looks good. Colour balance, spacing feel,
and "is this ugly" still need the user's eyes or a screenshot. Answer facts with
CDP; hand aesthetics to the user.

## When to Use

- Verifying a UI change actually took effect in the running dashboard
- "Why is this element still X?" — find the winning rule before editing anything
- Locating a stable selector for a component you're about to change
- Checking a design token's computed value on a real node
- Reading renderer console errors the user mentions but can't copy out

**Don't use for:** perf profiling or heap work (that's `node-inspect-debugger`),
or anything where the real question is "does this look right".

## Reading the DOM

Use the `browser` tool against the dashboard URL — it drives the same Obscura
CDP browser the web tooling shares:

```text
browser(command="navigate", url="http://127.0.0.1:9119/")
browser(command="accessibility_tree")     # structured tree (compact or full)
browser(command="snapshot")               # DOM text snapshot
```

For computed styles / "which rule won" questions, evaluate JS against the live
page via the CDP session (the `browser` tool's underlying WebSocket):

```js
// read computed values on a real node
const el = document.querySelector('[data-slot="assistant-message-root"] a')
JSON.stringify({
  ownClasses: el.className,
  weight: getComputedStyle(el).fontWeight,
  parents: (() => {
    const out = []
    let n = el
    while ((n = n.parentElement) && out.length < 6) out.push(n.className)
    return out
  })()
})
```

## The question this is best at: which rule won?

Editing every call site because a style "isn't applying" is the classic waste.
Read the real node first. If the node carries no class of its own, the value is
**inherited** — sweeping call sites will not fix it, and you need the ancestor
rule. A base stylesheet rule routinely beats a utility class; override on the
shared class, not at each usage.

## Running your own isolated instance

When no dashboard is running, or you must not disturb the user's session, start
a throwaway instance on a separate port with a separate home:

```bash
operant dashboard server --port 9333 --host 127.0.0.1
# or, for a fully isolated data dir:
HERMES_HOME=/tmp/cdp-probe-home operant dashboard server --port 9333
```

Then point the `browser` tool at `http://127.0.0.1:9333/`. Kill it when done.

## Pitfalls

- **Never kill the user's dashboard server or browser to "free" anything.** A
  mid-serve kill nukes the CDP socket pool, and the resulting network error
  gets blamed on whatever you just changed.
- **A throwaway `HERMES_HOME` has no real data.** API calls may error or be
  empty; the page still mounts and the DOM is readable. Read promptly.
- **Poll, don't probe once.** A just-launched server needs a second or two
  before it answers.
- **Never dump the whole DOM.** The dashboard renders hundreds of nodes and
  `outerHTML` will bury your context. Project down to a small JSON object inside
  the evaluated expression.
- **API calls need the bearer token** printed at dashboard startup — it is
  injected into the page as a global, so the browser tool reads the page state
  without it; direct curl API calls need `Authorization: Bearer <token>`.
