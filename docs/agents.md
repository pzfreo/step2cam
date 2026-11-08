# Agents in step2cam

How AI agents fit into step2cam's pipeline, and what they are not allowed to do.

## The principle: agents propose, code decides

step2cam's value is that every decision is traced to a rule and every toolpath is checked by
camlib. Agents keep that. An agent enters the pipeline the way a person would: it supplies a
decision, and the planner and camlib check it. Nothing an agent proposes reaches the plan without
passing the same checks as a rule's choice, and the trace records that the agent made it, and why.

Simulation is the stopping criterion. An agent loop ends when camlib accepts the plan, or after a
set number of tries, with what it tried reported.

This means:
- **Safety does not depend on the agent.** The checks are the product; an agent only chooses among
  what passes them.
- **Plans can be run again.** An agent's choices are stored in the plan, so running it again gives
  the same result without calling the agent.
- **Nothing is hidden.** The page shows each decision's source: a rule, the user, or an agent.

## Where agents fit

| Stage | What the agent does | What checks it | Issue |
|---|---|---|---|
| Before Recognise | Answers Specify's questions (material, hole functions, datums, fits) from drawings, notes and conversation, and asks the user the rest | specify-core | #61 |
| Recognise | Proposes how to group faces the rules can't classify | geometry checks (later) | — |
| Plan | Chooses among the options the rules accept: drill, bore or mill; raster or adaptive; which tool | the planner's rules | #58 |
| Plan | Proposes the setup order and the clamping in each setup | the planner's grip, reach and datum checks; camlib's fixture checks | #55 |
| Make | Repairs a camlib refusal by proposing another way, until camlib accepts | camlib | #60 |
| Learn | Turns a review comment into a rule change with a test | the tests and the benchmark (#53), then a person | #62 |
| Set up a shop | Builds the tool library and machine description from catalogues and manuals | the resources schema | #63 |

## What it needs

1. **Supplied decisions (#58).** One mechanism for any decision to come from outside, checked
   against the rules, recorded with its source and reason, and stored in the plan.
2. **Agent tools (#59).** specify-core, the planner and camlib behind one command-line and MCP
   interface, JSON in and out: analyse, answer, plan, simulate, benchmark, and post once camlib
   can post from Rust (stevecad/camlib#120).
3. **Where it runs.** Agents run outside the browser, through those tools (for example Claude
   with the MCP server). The page stays deterministic: it runs the plan, including any stored
   agent decisions, and shows who made each one.

## Order

1. #58 supplied decisions, with sources in the trace.
2. #59 agent tools.
3. #60 repairing refusals: the largest gain, since a refusal today leaves the feature uncut.
4. #61 answering Specify's questions.
5. #55 setup order and workholding (after #40, #43 and #45).
6. #62 rule changes from review comments.
7. #63 tool library and machine from catalogues.

Recognition by an agent comes last: it is the hardest to check.
