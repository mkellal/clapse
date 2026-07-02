# Clapse

Terminal-based C++ build profiling tool. Parses Clang `-ftime-trace` JSON output into an interactive flamegraph TUI for identifying compilation bottlenecks.

## Overview

Clapse ingests `-ftime-trace` JSON files produced by a C++ build, constructs a hierarchical span model, and renders an interactive, zoomable flamegraph in the terminal. It surfaces per-file aggregates, template instantiation hotspots, and suggests PCH/extern template candidates.

## Features

- **Flamegraph tab** — hierarchical time-aligned tracks of all compilation spans. Zoom/pan, click spans for details.
- **Sources tab** — spans aggregated by source file. Identifies top PCH candidates ranked by cumulative parse time (copiable as code).
- **Templates tab** — spans aggregated by template instantiation. Lists top slowest concrete instantiations as `extern template` candidates (copiable as code).
- **Search** — full-text search across all spans. `Enter` to seek, `n`/`p` to jump through matches.
- **Keyboard + mouse** — arrow keys, scroll wheel, click-to-select. `?` shows contextual keybindings.

## Requirements

- Rust 1.82+ (edition 2024)
- A C++ build directory containing `-ftime-trace` JSON files (file name pattern: `*.*.json`, paired with a corresponding `.o` file)

Enable `-ftime-trace` in your build:

```sh
cmake -DCMAKE_CXX_FLAGS="-ftime-trace" ..
```

## Installation

```sh
cargo install --path .
```

Or build and run directly:

```sh
cargo build --release
./target/release/clapse <build-dir>
```

## Usage

```sh
clapse <build-dir>          # Open the TUI on the given build directory
clapse <build-dir> --verbose # Enable verbose JSON parsing output
```

### Keybindings

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit |
| `?` | Toggle help popup |
| `s` | Open search |
| `Alt+1/2/3` | Jump to tab |
| `Alt+t` | Rotate to next tab |
| `↑` / `↓` | Navigate to parent / child span |
| `←` / `→` | Navigate to previous / next sibling |
| `Ctrl+↑` / `Ctrl+↓` | Zoom in / out |
| `+` / `-` | Zoom in / out (alternate) |
| `Ctrl+←` / `Ctrl+→` | Pan left / right |
| `PageUp` / `PageDown` | Zoom in / out fast (×2) |
| `Space` | Zoom to selected span |
| `r` | Reset zoom & pan |
| `Tab` / `Shift+Tab` | Next / previous track |
| `y` | Copy selected span identifier to clipboard |
| `m` | Toggle sort mode (flamegraph tab) |
| `Ctrl+Y` | Copy candidates to clipboard (sources/templates) |
| `Esc` | Deselect span / close search |
| Click | Select span, show details |
| Scroll wheel | Vertical scroll |

## Architecture

```
src/
├── main.rs          # Entry point, terminal setup
├── cli.rs           # CLI argument parsing (clap)
├── traces/
│   ├── event.rs     # -ftime-trace JSON deserialization (serde)
│   └── file.rs      # Glob-based trace file discovery
├── app/
│   ├── mod.rs       # App state, event loop, tab routing
│   ├── span.rs      # Span IR (type, timing, hierarchy)
│   ├── view.rs      # Track scheduling, ordering, span→screen mapping
│   ├── search.rs    # Full-text search state
│   ├── help.rs      # Help popup widget
│   └── tabs/
│       ├── flamegraph.rs  # Raw hierarchical flamegraph
│       ├── sources.rs     # Source-file aggregates + PCH candidates
│       └── templates.rs   # Template aggregates + extern candidates
└── widgets/
    ├── flamegraph.rs      # Flamegraph renderer (Unicode block chars)
    ├── track.rs           # Single-track rendering
    ├── span.rs            # Individual span cell rendering
    ├── span_details.rs    # Span detail panel
    ├── pch_candidates.rs  # Candidate list with copy-to-clipboard
    ├── time_range.rs      # Time-axis ruler
    ├── color.rs           # Catppuccin palette + depth-based shading
    └── start_screen.rs    # Loading / empty-state screen
```

## License

MIT
