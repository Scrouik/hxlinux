# HXLinux

<img width="1364" height="1009" alt="HXlinux" src="https://github.com/user-attachments/assets/b7e1a6a8-fca6-41cd-a1f6-95204f866c10" />


**EN:** Open-source **HX Stomp XL** editor for **Linux** (Tauri — Rust + TypeScript).  
**FR:** Éditeur open source pour **HX Stomp XL** sous **Linux** (Tauri — Rust + TypeScript).

Connect over native USB, browse and edit presets, assign FX blocks, tweak parameters live, and manage the signal chain — **without HX Edit**.

> **First usable version · Première version utilisable** (June 2026) — early testers welcome · testeurs bienvenus.

| | |
|---|---|
| **Features (FR)** | [docs/features-v1.md](docs/features-v1.md) |
| **Features (EN)** | [docs/features-v1.en.md](docs/features-v1.en.md) |
| **Install (FR)** | [docs/install.md](docs/install.md) |
| **Install (EN)** | [docs/install.en.md](docs/install.en.md) |

## Download · Télécharger

**[GitHub Releases](https://github.com/Scrouik/hxlinux/releases)** — AppImage or `.deb` (Linux x86_64).

After download, install the USB udev rule — see [install.en.md](docs/install.en.md) / [install.md](docs/install.md).

## Highlights · En bref

- Native USB to HX Stomp XL (125 presets: browse, activate, rename, save)
- Stomp matrix: copy / paste / move FX blocks, model picker, live parameter editing
- Amp+Cab and Cab Dual tabs, Path 1 Input & Split (live write + hardware scroll)
- Model metadata bundled — **HX Edit not required** for the release build

## Requirements · Prérequis

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
- **Protocol:** reverse-engineering started with [kempline/helix_usb](https://github.com/kempline/helix_usb)

## Documentation

| Doc | Content |
|-----|---------|
| [matrix-edit-handoff.md](docs/matrix-edit-handoff.md) | Matrix editing architecture |
| [Référence protocole USB](docs/Référence%20protocole%20USB%20HX%20Stomp%20XL.md) | USB protocol notes (FR) |

> [!WARNING]
> **Early release — back up your presets first**  
> Export your presets from HX Edit before using HXLinux.  
> No data loss has been reported, but this is a first release — better safe than sorry.
>
> **Première version — sauvegardez vos presets avant tout**  
> Exportez vos presets depuis HX Edit avant d'utiliser HXLinux.  
> Aucune perte de données n'a été signalée, mais c'est une première version — mieux vaut prévenir que guérir.
