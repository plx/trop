---
description: Verify that a phase is complete
argument-hint: [description of the phase (or where to find a description)]
disable-model-invocation: true
---

At this point we *think* we're done with a major phase of work, and want to verify that it's actually complete. This task *must* be delegated to the `phase-completion-verifier` agent: that agent is *extremely* specified to perform this exact task, and needs only minimal guidance beyond a description of what it's supposed to verify.

As such, you should simply invoke the `phase-completion-verifier` agent and ask it to verify the current phase's plan; here's the description of that plan (or where to find it):

$ARGUMENTS
