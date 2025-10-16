---
description: Verifies the code is suitable for opening a PR.
disable-model-invocation: false
---

# Overview

We're going to run our "preflight-pr" command, which consolidates the following tasks:

- does the code compile?
- is it formatting correctly?
- does clippy have any suggestions?
- do the tests pass?

If all of those succeed, we're good—you can simply announce that we're ready to open a PR.

If any of those fail, you should investigate the issue and attempt to fix (delegating to suitable agents as necessary).

Without further ado, here's the results of our preflight checks:

!`just preflight-pr`
