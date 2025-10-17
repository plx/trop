# `trop`

`trop` is a CLI tool that will be written in `rust`.

The high-level specification is in `specifications/ImplementationSpecification.md` and the high-level implementation plan is in `specifications/ImplementationPlan.md`.

During this initial development phase, the we should treat the `specifications/ImplementationSpecification.md` as the source of truth and **NEVER** modify it.

Development will generally follow the plan outlined in `specifications/ImplementationPlan.md`, although expect ad-hoc directives mixed in with significant work.

Finally, this repository is intended to produce the `trop` tool described in the specification, but it's also an experiment in "hands off", "high-autonomy" agentic delegation: the repo contains multiple agents and slash commands meant to facilitate tackling major blocks of work in a "one-shot-ish" fashion, and agents should keep that meta-goal in mind as they operate.
