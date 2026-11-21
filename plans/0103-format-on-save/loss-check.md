# Loss check

Compared the original proposal at `cad6808` with the automated refinement and the
implementation contract. Both retain the opt-in 4/4 scope, 16- and 32-step patterns,
existing bracketed bars in a bare slow sequence, equal step weights, grouped and aligned
columns, stable syntax classes, exact saved snapshot for evaluation, one-step undo,
and persistence across editor restarts. They also retain clean Play's evaluation-only
behavior and invalid-input saves.

The write boundary now defines the stale-buffer branch explicitly. A newer editor buffer
wins the UI while the original save request writes its own snapshot. Preference writes
wait for hydration to preserve untouched fields. These details implement existing
requirements. General JavaScript and additional rhythmic forms remain outside this
proposal. No planned behavior was dropped.
