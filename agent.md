# AGENT.md

## Project Overview

HXLinux is a Tauri 2 desktop application (Rust backend + TypeScript/Vite frontend) for editing presets on a Line 6 HX Stomp XL guitar processor via USB on Linux. The USB protocol was reverse-engineered from [kempline/helix_usb](https://github.com/kempline/helix_usb).

**Status:** Work in progress — preset names are readable; full preset parameter reading/editing is in progress.

**Runtime requirement:** HX Edit must be installed (provides `.models` files). Device VID/PID: `0x0e41` / `0x4253`.

## Commands

```bash
# Development
npm run tauri dev        # Launch full app (frontend + Rust backend)

# Init USB timeline (correlate with Wireshark): HW off → start app + capture → power on HW
HX_INIT_TRACE=1 npm run tauri dev
# Optional log file:
HX_INIT_TRACE=1 HX_INIT_TRACE_FILE=/tmp/hxlinux-init.trace npm run tauri dev
npm run dev              # Vite dev server only (port 1420, HMR on 1421)

# Build
npm run build            # TypeScript compile + Vite bundle
npm run tauri build      # Production binary

# Rust only (from src-tauri/)
cargo build
cargo check
cargo clippy
```

No test suite is currently defined.

## Architecture

### Threading model (Rust)

`lib.rs` spawns several threads at startup that communicate via `mpsc` channels:

- **USB monitor** (`usb_monitor.rs`) — watches for device connect/disconnect (libusb hotplug)
- **USB writer** (`usb_writer.rs`) — sends packets to endpoint `0x01`; receives `ModeRequest` variants from other threads via channel
- **USB listener** (`usb_listener.rs`) — reads from endpoint `0x81`; dispatches responses to the active mode
- **MIDI listener** — reads endpoint `0x82` for real-time preset change events
- **KeepAliveManager** (`keep_alive.rs`) — sends periodic X1/X2/X80 keep-alive commands

### Mode state machine (Rust)

`helix/mod.rs` defines a `Mode` trait. The active mode is stored in `HelixState` (behind `Arc<Mutex<>>`). Modes handle incoming USB packets and request transitions via `ModeRequest` enum sent on the writer channel. Mode sequence on startup:

1. `connect.rs` — USB handshake
2. `request_preset_names.rs` — bulk read of all 124 preset names
3. `standard.rs` — normal operating state (keep-alive, MIDI listening)

Single-preset and reconfigure modes exist as `request_preset_name.rs`, `request_preset.rs`, `reconfigure_x1.rs`.

### Tauri commands (Rust → TypeScript)

Defined in `lib.rs` with `#[tauri::command]`:
- `get_preset_names()` — returns current preset name list from `AppState`
- `get_active_preset()` — returns active preset index
- `rename_preset(index, name)` — sends rename packet to device

`AppState` holds `preset_names: Vec<String>`, `active_preset: usize`, and an `Arc<Mutex<HelixState>>`.

### Frontend (`src/main.ts`)

Single-file TypeScript with no framework. Polls Tauri commands every 1.5 s to refresh the preset list. Handles:
- Drag-and-drop preset reordering
- F2 / double-click rename
- Right-click context menu (Rename, Save to disk, Load from disk)
- Connection status display (connected / waiting / loading)

Code comments are primarily in French.
