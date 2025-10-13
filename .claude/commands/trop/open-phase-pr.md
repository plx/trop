---
description: Open a PR for human review; wind down work on this phase.
argument-hint: [description of the work done]
disable-model-invocation: true
---

# Overview

We've come to the end of the previous phase, we have verified we've completed what we set out to do, and so now it's time to open a PR for human review. In addition to opening the PR, itself, this is when we're going to prepare the following:

- updates to the memory file (e.g. CLAUDE.md updates)
- creating-or-appending to a `DevelopmentLog.md` file 
- (if necessary) updates to *future steps* in the `ImplementationPlan.md`
- writing out any topic-specific documentation or guides (e.g. if we discovered some widely-applicable rust pitfall, helpful design pattern, or similar, consider adding a writeup in `guides`—creating the directory if necessary)

The reason we do these tasks at this stage is because we want their substance to be reviewed alongside the code changes motivating them; I'll describe them first and then describe the PR process afterwards.

## Auxiliary Tasks

### Memory File Updates

We want to keep our CLAUDE.md file up-to-date, but also minimal and low-context; as such, the kinds of updates we should consider include:

- major changes to tooling, project structure, other infrastructure, and so on
- discovery of some patterns or best practices that are widely applicable (e.g. should be "top-of-mind" for future work)

Be judicious, here—"would this merit inclusino every time, for every agent, doing every task?"

### Development Log Updates

The development log is a running record of the work done on the project, but is not truly a "work log" per se; instead, it's better understood as a mechanism by which we can gather information about how well the workflow is (or isn't!) working out for us. As such, entries to the work log should include *just enough detail* to understand the work being tackled...and should mostly note when things needed multiple revisions, iteration, places progress almost got stuck, and so on.

Entries should look like:

```md
## YYYY-MM-DD - $Task Description

One paragraph about what we did, and zero-to-two additional paragraphs about the process (including particular focus on pain points or challenging bits).
```

### Updating Future Steps

If the work we did in this phase has bearing on future steps, we should update the `ImplementationPlan.md` to reflect that. Note that we *do not* want to update that document for minor changes—if it's "arguably still implied" by the existing plan, no need to change it! The kinds of things that *do* merit an update include:

- modifying the order of future steps
- adding or removing CLI arguments (or other major capabilities)
- needing to redesign core product aspects (e.g. due to discovered limitations)

When updates are necessary, preserve the original structure and "tone", and do not explain the changes—just make the relevant changes.

### Writing Documentation

Generally, you shouldn't do this proactively—when in doubt, skip it.

The exception would be something like: "it turns out that this feature in `clap` *seems like* it would let you do X, and it kind-of-does, but not in the way we'd need for its "X" to be the "X" we need; instead, you'll need to do Y, even though Y looks clunkier and X really seems like it should work." 

If it's that tier of issue, write a guide...but, if not, skip it.

## PR Details

If the repo has a PR template (`.github/PULL_REQUEST_TEMPLATE.md`), use that; otherwise, use the following strategy:

- the title should be a human readable description of the phase ("Setup project" or "Add `reserve` Subcommand")
- the body should include:
  - a brief human-readable summary of the phase goals (e.g. taken from the `ImplementationPlan.md` or phase-specific plan)
  - a brief human-readable summary of the work done, in enough detail to facilitate the review:
    - describe any *major* types or functions introduced or significantly modified
    - describe the testing strategy employed
    - describe any significant modifications to previous work 
    - describe any significant changes to the project's structure or organization
    - describe any significant changes vis-a-vis the original implementation plan
  - call out anything deserving special attention (tricky or subtle logic, etc.)

Once the PR is opened, consider adding proactive self-review comments to the PR, but be selective:

- new code shouldn't need comments *unless* it's tied to some "change of plans" (e.g. we discovered a limitation and had to modify the plan—in that case, flag the parts corresponding to the modification)
- modifications to existing code *should* be commented, but only when the purpose of the modification isn't immediately obvious (keeping in mind we'll have rich context from the substance of the PR itself)
- modifications to the plan, CLAUDE.md, etc., should generally be commented with an explanation and justification; the one exception are *trivial* changes (e.g. fixing spelling errors or performing "pure-rename" operations)

## Work Description

$ARGUMENTS
