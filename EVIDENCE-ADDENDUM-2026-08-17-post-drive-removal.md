# Addendum to test-to-dev/aismartguy-end-to-end-2026-08-17/README.md

Written locally because the shared drive was removed before this could be
copied to `test-to-dev/`. Sync this file into that folder (and fold it into
README.md finding #7) next time the drive is available.

## Finding #7 update: live GUI — now partially confirmed via a real click

The box rebooted mid-session (uptime ~5 min when noticed), which killed the
live app process and wiped `/tmp` — unrelated to anything in the app itself.
Relaunched the app fresh afterward; it survived the drive already being gone
without crashing, consistent with the 2026-08-04 handoff's documented
"unplugged drive must not brick the app" behavior for `model_library_dir()`.

With the app live again, the operator did real physical clicks:

1. **PDF drag-and-drop: CONFIRMED WORKING via a real drop.** The app's
   stdout genuinely emitted `{"event":"progress","data":{"stage":"LoadingPdf",...}}`
   then `{"stage":"ExtractingMetadata",...}` — the exact real sequence
   `ui::commands::load_pdf` emits, only reachable through the real Tauri
   drag-drop event handler. This is upgraded from "not verified via a real
   click" to **verified live**.
2. **📁 Browse… (model library) button: CONFIRMED WORKING via a real
   click.** Opened a real native folder picker (rfd), operator navigated it
   and correctly found no model present. That's expected, not a bug — the
   real model file only ever existed on the now-removed drive, at a path
   (`~/.aismartguy/model_library/`) the GUI's own default `model_library_dir()`
   never pointed to (that path was specific to this session's direct-harness
   testing, not the app's real default `~/.local/share/com.aismartguy.app/models`,
   which is genuinely empty).
3. **Open Output 📂 button: exists, correctly gated, NOT click-tested.**
   Traced in `frontend/index.html`: `#procOpenBtn` ("Open Output Folder")
   lives inside `#procComplete`, `hidden` by default, only revealed once a
   run actually completes (the processing screen's "Report Complete"
   state). This is correct, intentional UX — not a bug — but it means the
   only way to click-test it live is to actually finish a run in the GUI,
   which needs the model, which needs the drive. Not tested tonight.
   **Doc drift, not a functional bug**: handoffs.md's 2026-08-04 entry
   describes this button as living "next to Lock for 10 Runs" (a different
   screen, the model-selection/gating screen at `#mlLockBtn`) — the current
   source doesn't match that description. Worth a quick doc fix, not a code
   fix.

## Net effect on finding #7's status

Was: "PENDING — asked the user, not confirmed." Now: PDF drag-and-drop and
the Browse button are genuinely verified via real physical clicks; the Open
Output button remains unverified live (correctly gated behind a completed
run that needs the now-unavailable model) but its code path was already
proven real via the harness's direct `open_in_file_manager`/xdg-open
resolution check earlier tonight.
