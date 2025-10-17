---
description: Implement an existing plan
argument-hint: [description of the plan or where it can be found]
disable-model-invocation: true
---

We're going to implement a bit of work that has already been planned-out. 

## Implementation Guidelines

Use your best judgment for how to tackle the planned work, but please do take the following guidelines into account:

- if it's "project administration" work, have the `project-administrivia-drone` agent tackle it 
- if it's more-involved github-actions work, have the `github-actions-specialist` agent  tackle it 

Additionally, for general-purpose development work, you're encouraged to use a strategy along the following lines:

- have the initial development done by the `rust-implementer` agent
- once the initial work is done, have the `rust-test-writer` agent write an expanded suite of manual tests, as appropriate for the specific task:
  - unit tests for datatypes and internal routines (or changes to them)
  - integration tests for CLI subcommands (or changes to them) 
- *where appropriate*, have the `property-test-writer` agent write property tests)
  - these should be written *after* the initial implementation and test suite exists
  - these are appropriate for "basic building blocks" (types, methods, etc.) but not necessarily for higher-level concepts like CLI subcommands

In other words, you want to get the initial work done and provisionally-correct, then verify the correctness by adding a suite of "hand-written" tests, and then—where appropriate—further verify the correctness by introducing additional property tests, too. What this general concept translates into for a specific task will vary, and you should trust your judgement as to how to adapt this concept to a specific task.

Finally, in addition to those agents, there are two narrow specialists you may want to consider:

- the `test-failure-investigator` can be invoked to investigate complex and-or-unexpected test failures (e.g. when testing unveils an unanticipated edge case for which no immediately-obvious fix is readily apparent)
- the `rust-compilation-fixer` can be invoked to fix not only compilation errors, but also e.g. lint errors, clippy warnings, and so on; *generally* the `rust-implementer` agent should be writing code that compiles and passes all tests, etc., but the `rust-compilation-fixer` agent exists to bring in on hard cases

As noted above, you should do what makes sense for the specific task at hand, while keeping the plan outlined above in mind as a generally-reasonable approach (e.g. as a suitable starting point for adapting to specific tasks).

## Implementation Request

$ARGUMENTS
