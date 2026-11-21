# App integration and validation

Goal. Wire typed note events to the preview while transport and other audio are idle.

Prerequisite. Editor candidate, checkbox and preview APIs with focused tests.

Compose the player in the app, sharing runtime initialization and invalidating stale work across application and editor transitions. Verify play and stop, document replacement, recording, memo playback, and teardown. Run the canonical frontend and Rust gates and a desktop smoke test. Check the visible checkbox and exercise a G3 preview using the app. Record platform or validation gaps.

Commit as `feat(editor): audition completed notes while stopped`.
