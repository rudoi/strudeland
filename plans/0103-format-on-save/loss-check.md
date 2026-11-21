# Loss check

Compared the original proposal at `46d3c34` with the automated refinement and the
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

For a slow sequence the first release lays out all bars together when each is a supported
16- or 32-step bracketed bar. A sequence mixing a supported bar with another form retains its
existing layout so the formatter does not relocate the other bars.
