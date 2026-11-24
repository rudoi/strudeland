# Live reverb plan verification

The [proposal](proposal.md) retains private-only hardware comparison, attribution of worker
construction, render-thread installation and steady processing, and a minimized synthetic
48 kHz regression. Its production references cover the callback, IR worker, request application,
and sample processing. The existing dropout suite exercises the production player with a fake
device, but its final stats interval alone cannot describe minimum fill over a full session.

Refinement adds checks for a cold FFT plan after cloning, a fully active wet tail, output
timing across any convolution boundary, and immediate warm switches. These checks address the
engine's existing newest-request and history semantics. No original requirement was removed.
