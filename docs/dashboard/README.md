# conductr · dashboard (mockup)

`docs/dashboard/index.html` is a **static visual sketch** of a future
dashboard UI for conductr. It is **not wired** to anything — open the file
in a browser to see the layout, nothing more.

## Status

- No UI exists today. `conductr` is driven by:
  1. `crates/conductr` — the CLI binary
  2. `skills/*` — markdown skills invoked inside Claude
  3. cron (installed by `conductr begin`)
- A dashboard would be a **third driver** in the hexagonal layout, sitting
  alongside the CLI and skills. It would call the same use-case crates
  (`conductr-orchestrate`, `conductr-pod`, `conductr-tasks`, …) — no new
  port traits are required for the mockup.

## The mockup

Styled like a MIDI synth control surface:

- **Master fader · Safety.** A 7-detent vertical fader from `UNHINGED` to
  `BUREAUCRATIC`. Defines the global safety posture; channel strips
  inherit it unless overridden.
- **Channel strips · per routine.** One strip each for `orchestrate`,
  `idle`, `architect`, `pod`. Each has:
  - arm / activity LEDs
  - a mini-fader for a per-routine safety override
  - an on/off toggle
  - an LCD-style readout showing the channel's cron or status
- **Preset bank.** Six preset buttons (A1/A2/B1/B2/C1/C2) — these will
  be the named safety levels once we've scoped them.
- **Branch isolation knob** + **red-CI merge tolerance meters.**
  Visual placeholders for the two axes we still need to define.

## What needs scoping (interviews)

The mockup deliberately leaves the **levels themselves** undefined.
Three `human`-labelled GitHub issues track the open questions:

- _Define safety presets — how branches should consider each other._
- _Define safety presets — how much red CI a preset can merge._
- _Name & document the preset bank (UNHINGED → BUREAUCRATIC)._

Resolve these first; only then is it worth picking a stack (Leptos /
Dioxus / plain HTMX / Tauri) and wiring the dashboard into the
use-case crates.
