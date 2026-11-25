# Help bindings

**Goal.** Resolve the active accessible control and display its help without changing behavior.

**Prerequisites.** [Help interaction](help-interaction.md).

**Implementation.** Build the owner policy, window listeners, DOM bindings, and Help component. Add help props to the shared primitives.

**Validation.** Policy tests, description merging, target resolution, disabled hover, scope reset and a real browser transition.

**Commit.** feat(ui): add shared contextual help bindings
