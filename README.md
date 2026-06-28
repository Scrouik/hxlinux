# HXLinux

Open-source **HX Stomp XL** editor for **Linux**, built with Tauri (Rust + TypeScript).

Connect over native USB, browse and edit presets, assign FX blocks, tweak parameters live, and manage the signal chain — without HX Edit.

## Status

**First usable version** (June 2026) — early testers welcome.

| | |
|---|---|
| **Full feature list (FR)** | [docs/features-v1.md](docs/features-v1.md) |
| **Full feature list (EN)** | [docs/features-v1.en.md](docs/features-v1.en.md) |
| **Install (testers)** | [docs/install.en.md](docs/install.en.md) |

### Highlights

- Native USB to HX Stomp XL (125 presets: browse, activate, rename, save)
- Stomp matrix: copy / paste / move FX blocks, model picker, live parameter editing
- Amp+Cab and Cab Dual tabs, Path 1 Input & Split (live write + hardware scroll)
- Model metadata bundled — **HX Edit not required** to run the release build

### Not yet

- Helix LT / Floor editing, DSP budget, preset file import/export, preset reorder on device

## Download

**[GitHub Releases](https://github.com/Scrouik/hxlinux/releases)** — AppImage or `.deb` (Linux x86_64).

Quick start after download: [docs/install.en.md](docs/install.en.md) (USB udev rule required).

## Requirements

- Linux x86_64 (Ubuntu/Debian family tested)
- Line 6 **HX Stomp XL** via USB
- udev rule: [`packaging/99-line6-helix.rules`](packaging/99-line6-helix.rules)

## Development

```bash
npm ci
npm run tauri dev    # desktop app
npm run tauri build  # release bundles in src-tauri/target/release/bundle/
```

## Tech stack

- **Backend:** Rust, Tauri 2, `rusb`
- **Frontend:** TypeScript, Vite
- **Protocol:** reverse-engineering inspired by [kempline/helix_usb](https://github.com/kempline/helix_usb)

## Documentation

| Doc | Content |
|-----|---------|
| [description.md](description.md) | Session handoff memo |
| [TODO.md](TODO.md) | Backlog |
| [matrix-edit-handoff.md](docs/matrix-edit-handoff.md) | Matrix editing architecture |

## License

See repository license file if present; otherwise check with the maintainer.
