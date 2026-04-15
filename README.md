# HXLinux

Open source HX Stomp XL editor for Linux, built with Tauri (Rust + TypeScript).

## Status

Work in progress, but already usable for core preset browsing tasks.

### Working
- Native USB connection to HX Stomp XL on Linux
- Device handshake and mode transitions
- Reading all preset names from the device (125 slots)
- Main window with preset list and rename support
- Preset activation from the UI
- Dedicated models window (`models.html`) that opens at startup
- Reliable preset-content loading in the models window when switching presets

### In progress
- Deeper preset parameter decoding and mapping
- Better signal chain visualization
- Real-time parameter editing

### Planned
- Parameter controls generated from Line 6 model metadata
- Export/import for preset files
- Additional UX polish and editor ergonomics

## Tech stack

- **Backend:** Rust, Tauri 2, `rusb`
- **Frontend:** TypeScript, Vite (multi-page: main + models window)
- **Protocol basis:** reverse-engineering work inspired by Kempline

## Requirements

- Linux (tested on Ubuntu/Debian family)
- Line 6 HX Stomp XL connected via USB
- HX Edit installed (to provide model metadata files)

## Run

```bash
# Frontend dev server only
npm run dev

# Full desktop app (frontend + Tauri backend)
npm run tauri dev
```

## Build

```bash
# Frontend build
npm run build

# Desktop app production build
npm run tauri build
```

## Project notes

- The app uses a Rust mode state machine to manage USB protocol phases.
- USB communication is asynchronous (listener/writer threads + channels).
- The models view is intentionally split into a second window to keep the main UI simple.

## Credits

USB protocol reverse engineering inspired by:
[kempline/helix_usb](https://github.com/kempline/helix_usb)
