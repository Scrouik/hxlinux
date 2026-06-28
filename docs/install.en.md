# HXLinux installation (testers)

> French version: [install.md](install.md)

## Download

Get the latest release on GitHub: **Releases** → **AppImage** or **.deb** asset (Linux x86_64).

Recommended tag for the first usable build: `v0.1.0` (pre-release).

## Install

### AppImage (recommended — any distro)

```bash
chmod +x hxlinux_*_amd64.AppImage
./hxlinux_*_amd64.AppImage
```

### Debian / Ubuntu (.deb)

```bash
sudo dpkg -i hxlinux_*_amd64.deb
hxlinux
```

## USB access (required)

Without a udev rule, the app cannot open the Helix (or you would need `sudo`).

```bash
sudo cp packaging/99-line6-helix.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Unplug/replug the Stomp XL. Your user must be in the `plugdev` group (default on Ubuntu).

## Requirements

| Item | Detail |
|------|--------|
| OS | Linux x86_64 (tested on Ubuntu/Debian) |
| Hardware | **HX Stomp XL** (only fully validated device) |
| HX Edit | **Not required** for normal use — model metadata is bundled |

## First run

1. Connect the Stomp XL via USB.
2. Launch HXLinux — preset list + models matrix open.
3. Wait for connection (green status) and active preset load.

## Known limits (v0.1)

- Helix LT / Floor: may be detected; editing not supported.
- Preset list reorder: UI only (not sent to HX yet).
- Load preset from disk: not implemented.
- See [features-v1.en.md](features-v1.en.md) for the full feature list.

## Build from source

```bash
git clone https://github.com/Scrouik/hxlinux.git
cd hxlinux
npm ci
npm run tauri build
```

Artifacts: `src-tauri/target/release/bundle/`.

Build dependencies (Debian/Ubuntu):

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libudev-dev
```
