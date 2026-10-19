<!-- genome:zone records sha=116ef188ce28 -->
**Decisions** (10)

| ID | Title | Status | Description |
|---|---|---|---|
| AD-001 | Body runs write no sessions row; attribution lives in the trajectory | Active | Body runs write no sessions row; attribution lives in the trajectory |
| AD-002 | The TUI org view reads the Holon read API in-process, not by shelling out | Active | The TUI org view reads the Holon read API in-process, not by shelling out |
| AD-003 | The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver | Active | The TUI keeps the default operant home; HERMES_HOME belongs to the Holon driver |
| AD-004 | Org mode validates /session employee ids against the Holon roster before binding | Active | Org mode validates /session employee ids against the Holon roster before binding |
| AD-005 | Operant plugins extend through one TOML manifest; permissions are enforced and tool and LLM hooks ship before a guest SDK | Proposed | Plugin ecosystem architecture: one TOML manifest, enforced permissions, tool-call and LLM-request hooks, config-owned enable state, and a git-first install path |
| AD-006 | Holon bodies are fenced from the supervisor's secrets and their slot | Active | A Holon body cannot read the supervisor's environment, cannot inherit chair authority, and dies with its slot |
| AD-007 | A Holon run without policy.json is refused, not run unfenced | Active | A Holon run whose policy.json is missing fails to start, instead of running with no seat authority |
| AD-008 | A chair notice is retried until delivered; driver tokens are plain | Active | The Holon kernel retries a failed chair notice on the next heartbeat, and refuses driver placeholders that are not plain tokens |
| AD-009 | A gateway budget is metered per seat turn, and a hard cap fails closed | Active | Spend is recorded per turn against the start-time seat and the window reads that ledger; a hard cap whose usage cannot be read refuses |
| AD-010 | A chat's conversation is kept on its row; /new and a seat change start a fresh one | Active | The conversation id persists on the session row, /new and a seat change start a fresh transcript, the continuation summary is keyed by it, and seat runs set their memory scope |
<!-- genome:end records -->

<!-- genome:zone idmap sha=a7a72b4c9dd0 -->
(no retired records)
<!-- genome:end idmap -->
