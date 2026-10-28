# Graph Report - flamenote  (2026-10-28)

## Corpus Check
- cluster-only mode — file stats not available

## Summary
- 287 nodes · 564 edges · 14 communities (12 shown, 2 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 4 edges (avg confidence: 0.72)
- Token cost: 2,579 input · 1,566 output

## Graph Freshness
- Built from commit: `4a51373b`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- X11 Event Handling
- X11 Window Setup
- GitHub Actions Publishing
- Note Card UI
- Config and Hotkeys
- Application Event Loop
- Project Feature Overview
- X11 Window Relations
- Keyboard Modifier Bits
- Crosshair Icon Rendering
- Sample State Types
- Selection State Machine
- X11 Property Decoding
- Flamenote Project

## God Nodes (most connected - your core abstractions)
1. `X11` - 51 edges
2. `XResult` - 40 edges
3. `run()` - 17 edges
4. `App` - 14 edges
5. `AppEvent` - 12 edges
6. `Config` - 10 edges
7. `Command` - 10 edges
8. `spawn_note()` - 9 edges
9. `parse_hotkey()` - 9 edges
10. `selection_belongs_to_focus()` - 9 edges

## Surprising Connections (you probably didn't know these)
- `App` --references--> `Toolbox`  [INFERRED]
  src/main.rs → src/ui.rs
- `X11` --references--> `Config`  [EXTRACTED]
  src/x11svc.rs → src/config.rs
- `X11` --references--> `Hotkey`  [EXTRACTED]
  src/x11svc.rs → src/config.rs
- `run()` --references--> `Config`  [EXTRACTED]
  src/x11svc.rs → src/config.rs
- `spawn()` --references--> `Config`  [EXTRACTED]
  src/x11svc.rs → src/config.rs

## Import Cycles
- None detected.

## Hyperedges (group relationships)
- **X11 desktop integration stack** — readme_x11, readme_systray_xembed, readme_gtk3, readme_compositor [EXTRACTED 0.85]
- **Esc dismissal across application states** — readme_selection_mode, readme_toolbox, readme_note_card [EXTRACTED 0.90]
- **Text selection to note flow** — readme_hotkey, readme_selection_mode, readme_toolbox, readme_primary_selection, readme_note_card [EXTRACTED 0.95]

## Communities (14 total, 2 thin omitted)

### Community 0 - "X11 Event Handling"
Cohesion: 0.13
Nodes (8): chord_matches(), debug(), keysyms_for_name(), modifier_variants(), plain_escape(), run(), X11, XResult

### Community 1 - "X11 Window Setup"
Cohesion: 0.04
Nodes (16): ARM_QUERY, BUTTON1, CONFIG_POLL, CROSS, CROSS_HOT, DOCK_RETRY, EMPTY_RELEASE_TRIES, ICON_PX (+8 more)

### Community 2 - "GitHub Actions Publishing"
Cohesion: 0.06
Nodes (39): container, name, runs-on, steps, AUR_SSH_PRIVATE_KEY, REPOSITORY_TOKEN, jobs, build (+31 more)

### Community 3 - "Note Card UI"
Cohesion: 0.11
Nodes (26): apply_rounded_shape(), build_toolbox(), button_rect(), CARD_PAD, clamp(), Drag, monitor_at(), paint_note() (+18 more)

### Community 4 - "Config and Hotkeys"
Cohesion: 0.09
Nodes (24): Config, config_path(), ConfigError, CONTROL, DEFAULT_HOTKEY, display_key(), FileConfig, home_dir() (+16 more)

### Community 5 - "Application Event Loop"
Cohesion: 0.10
Nodes (19): App, capture(), instance_lock(), main(), on_event(), popup_menu(), show_toolbox(), start() (+11 more)

### Community 6 - "Project Feature Overview"
Cohesion: 0.23
Nodes (13): Compositor (e.g. picom), config.toml, flamenote, Flameshot, GTK 3, Global hotkey, Single-instance lock, Note card (bilhete) (+5 more)

### Community 7 - "X11 Window Relations"
Cohesion: 0.40
Nodes (7): Atoms, intern(), is_ancestor(), same_x_client(), selection_belongs_to_focus(), window_pid(), windows_related()

### Community 8 - "Keyboard Modifier Bits"
Cohesion: 0.33
Nodes (4): IntoKeyBits, key_state_bits(), u16, x11rb::protocol::xproto::KeyButMask

### Community 9 - "Crosshair Icon Rendering"
Cohesion: 0.40
Nodes (4): in_round(), render_crosshair(), render_icon(), write_pixel()

### Community 10 - "Sample State Types"
Cohesion: 0.40
Nodes (5): SampleKind, AfterRelease, Baseline, Normal, Stale

### Community 11 - "Selection State Machine"
Cohesion: 0.50
Nodes (4): SelState, AwaitIncr, AwaitNotify, Idle

## Knowledge Gaps
- **15 isolated node(s):** `AfterRelease`, `Baseline`, `Normal`, `Stale`, `AwaitIncr` (+10 more)
  These have ≤1 connection - possible missing edges or undocumented components. (Counts symbols only; 118 node(s) total have ≤1 connection when file, concept and rationale nodes are included.)
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `X11` connect `X11 Event Handling` to `X11 Window Setup`, `Config and Hotkeys`, `Application Event Loop`, `X11 Window Relations`, `Crosshair Icon Rendering`, `Sample State Types`, `Selection State Machine`?**
  _High betweenness centrality (0.140) - this node is a cross-community bridge._
- **What connects `AfterRelease`, `Baseline`, `Normal` to the rest of the system?**
  _15 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `X11 Event Handling` be split into smaller, more focused modules?**
  _Cohesion score 0.12854609929078015 - nodes in this community are weakly interconnected._
- **Why does `Config` connect `Config and Hotkeys` to `X11 Event Handling`, `X11 Window Setup`, `Application Event Loop`?**
  _High betweenness centrality (0.089) - this node is a cross-community bridge._
- **Should `X11 Window Setup` be split into smaller, more focused modules?**
  _Cohesion score 0.043478260869565216 - nodes in this community are weakly interconnected._
- **Why does `Command` connect `Application Event Loop` to `X11 Event Handling`, `X11 Window Setup`?**
  _High betweenness centrality (0.077) - this node is a cross-community bridge._
- **Should `GitHub Actions Publishing` be split into smaller, more focused modules?**
  _Cohesion score 0.06153846153846154 - nodes in this community are weakly interconnected._