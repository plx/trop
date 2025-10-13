---
description: Request formulation of a plan to tackle a complex situation.
argument-hint: [description of the needed plan and how to communicate it]
disable-model-invocation: false
---

# Overview

We are going to craft a detailed plan to address a complex situation. Rather than simply "make a plan", the plan should be prepared via an iterative cycle.

Once ready, we're going to solicit feedback from human agent before proceeding, as a final "sign-off" before continuing work.

## Plan-Critic Workflow
 
The core plan-critic workflow looks like this:

- ask the `phase-planner` agent to prepare an initial plan
- ask the `plan-critic` agent to evaluate the initial plan
- if the `plan-critic` says it looks OK to proceed, we're ready to move forward
- if the `plan-critic` has substantive feedback, we should do a round of refinement:
  - ask `phase-planner` to prepare a revised plan (providing the original task, the initial plan, and the `plan-critic` agent's feedback
  - ask `plan-critic` to evaluate the revised plan (providing the original task, the initial plan, the original feedback, and the revised plan)
- if we're OK at this point, we're ready to move forward
- if the `plan-critic` still has substantive feedback, do another "refinement cycle"
- no matter what, we're done after this last cycle:
  - if the plan is OK, we're ready to move forward
  - if the plan still needs further work, report the situation to the primary agent (and thereby to the human operator)
  
In other words, do no more than 3 total rounds of "make-or-revise a plan, then solicit feedback"—if we can't make a suitable plan in 3 rounds, we should rethink what we're doing instead of continuing to spiral.

## Soliciting Human Feedback

If we fail to produce a plausible plan, we should inform the human operator and await further instructions.

If we *have* prodiced a plausible plan, we should solicit human feedback before proceeding. In particular, we should give the human a chance to review-and-edit the plan before proceeding; this should not be an extended back-and-forth, and can consist of a "please review and edit the plan at $path/to/plan.md, then let me know when to continue".

## Description Of Needed Plan

$ARGUMENTS
