# Security Policy

## Reporting a vulnerability

**Use GitHub's private vulnerability reporting.** On this repository:
**Security -> Report a vulnerability**. That opens a private advisory visible
only to the maintainers.

Please **do not** open a public issue for a security problem.

Include, if you can:

- what the vulnerability lets an attacker do (the impact, not just the crash),
- the exact steps or input that reproduces it,
- the version or commit you tested,
- what you already tried, so the fix can be checked against it.

### What to expect

| stage | what happens |
|---|---|
| acknowledgement | within **3 working days** |
| first assessment (severity, scope) | within **10 working days** |
| fix or a plan for one | communicated before the advisory is published |

If a report turns out not to be a vulnerability, you will be told why. That is
not a brush-off: the reasoning is written down so the next person can check it.

## What is in scope

- anything that lets a caller of the terminal, the gateway or the
  HTTP API reach a path, a credential or a channel it was not granted;
- anything that lets one session read another session's transcript, memory or
  credentials;
- the skill import path: a skill pack is executed with the agent's own
  privileges, so a pack that escapes its directory, or an integration
  tarball that is not the one its tag names, is in scope;
- the OAuth flows: a redirect that carries a session token in the URL is a
  leak, and the fix belongs in the flow rather than in the log;
- anything that lets a prompt or a fetched page steer the agent into
  running a command the operator did not ask for.

## What is out of scope

- **Findings that need an already-privileged position.** If you are already the
  operator of the machine this runs on, you do not need a vulnerability to act
  as it.
- **Denial of service against the operator's own machine** by exhausting the
  operator's own quota.
- **Social engineering of the operator.** The operator is a person; a person
  who is deceived has made a decision, and the fix is in the decision.
- Missing hardening suggestions with no demonstrated impact.
- Reports generated entirely by an automated scanner with no reproduction.

## Safe harbour

Good-faith research is welcome. You may read and run the code on a machine you
own, make requests to a deployment you own or have written permission to test,
and publish a proof of concept after the advisory is published, or after 90 days
if the maintainers have not responded.

You will not be pursued for harm to users or data, or for accidental damage,
provided you did not exfiltrate data, access accounts you did not have, or
degrade the service beyond what a normal user's activity would.

If you find something and are unsure whether it is a vulnerability, report it
privately and ask. "Is this a bug?" is not a reason to publish first.
