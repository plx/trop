---
description: Undertake another major phase of this project's implementation plan
argument-hint: [description of the phase (or where to find a description)]
disable-model-invocation: true
---

We're kicking off another major phase of our high-level implementation plan, and you're acting as the "primary agent": your job is to coordinate-and-orchestrate the work of the appropriate sub-agents to incrementally make your way through the "development workflow" described below.

As that "primary agent", your goal is to complete this phase end-to-end, with minimal need for human intervention—do not take shortcuts or accept "partial solutions", but *if you can* you should aim to work autonomously; success on that front looks like "the first thing I 'heard' from you was that you'd opened a PR with the complete work for this phase" (...and the PR needed only minor tweaks before it was ready to merge).

On the other hand, if you find yourself blocked or otherwise unable to proceed, you should try to identify a reasonable "stopping point", get to that point, and then communicate the situation to the human operator—use your best judgement.

## Preliminary Checks & Git Housekeeping

First—critically—before beginning you should create a new branch for the phase. If the user specifies a branch name, use that; if not, derive a three-to-five-word, human-readable name from the phase description, prefixed with `phase-$number` (e.g. `phase-00-setup-project` or `phase-07-add-reserve-subcommand`, etc.).

If the git status isn't *clean* enough to create a new branch, **DO NOT PROCEED**. Instead, loudly and directly communicate the situation to the human operator.

## Intended Workflow

Once we're on a dedicated branch, we're ready to proceed with the actual workflow.

### Prepare Implementation Plan

You should use the `/trop:prepare-plan $description` command to prepare an implementation plan for the phase we're undertaking. *Unless specified otherwise*, the implementation plan should be stored in a file named after the branch (e.g. `phase-00-setup-project.md`) and stored within a `plans/phases` directory (which should be created if necessary). 

Note that the `/trop:prepare-plan $description` command already encapsulates a plan-critic-revise cycle adapted for this project and its agents—if it provides a plan, you should treat that plan as "final and ready to go" (e.g. as needing no further evaluation).

### Implement Plan

Assuming we obtained a suitable plan, implement it by invoking the `/trop:implement-plan $description` slash command (and providing the path to the plan file produced in the previous step). Note that this slash command is deliberately written to execute plans within this specific project, using our project-specific agents—there should be little need to "steer" implementation beyond pointing it to the right plan file. 

### Verify Completion

Assuming the implementation was successful, you should use the `/trop:verify-completion $description` slash command before accepting that claim. That command contains detailed instructions for performing this completeness review, and so you should feel comfortable treating its results as definitive. 

That said, you will have to use your best judgement to decide how to handle its findings:

- small-and-isolated issues can be fixed without a full cycle
- larger, more-interwoven issues should be addressed via a new plan-and-implement cycle (e.g. using a similar workflow, but adapted to the specific situation)
- fundamental design issues or significant overisghts should be escalated to the human operator

### Open PR

Assuming the completion-verification is successful (e.g. "we think we're done"), your last step is to open a PR for human review. To do this, you should invoke the `/trop:open-phase-pr $suggested-title` slash command, providing a brief description of the overall phase (or the path to the plan file, etc.).

At that point, you're done—the human operator will review the PR and drive things forward from there (requesting fixes, merging it, and so on).

### Miscellaneous Points

This workflow is suitable for work that can be done in a single "implementation pass". If the work requires multiple passes, you should use your best judgement to adapt the workflow as needed, including by tasking the "implementation" steps with smaller portions of the work.

This workflow also only documents the "success" path (e.g. each step reaches a successful conclusion, even if it requires multiple passes before it gets there). As noted earlier, you should aim to finish the entire phase without human intervention, but only if it can be finished without cutting corners or accepting "partial solutions"—if you truly get blocked, find a reasonable stopping point and communicate the situation to the human operator.

## Phase Description

Here's the description of the phase we're undertaking:

$ARGUMENTS
