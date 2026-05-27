# Captures USB (Wireshark / USBPcap)

Exports JSON Wireshark (`usb.capdata`) pour reverse du protocole Helix — **hors** du crate Tauri.

## Emplacement

Ce dossier remplace l’ancien `src-tauri/paquets JSON/` (qui provoquait des rebuilds
lourds ou des blocages quand on y copiait de gros fichiers pendant `npm run tauri dev`).

## Usage

- Copier ici les nouvelles captures ; elles ne sont **pas** compilées ni bundlées.
- Fichiers en général **non versionnés** (voir `.gitignore`).
- Scripts :
  - `scripts/analyze_ed03_captures.py` — lanes ED03 / preset
  - `scripts/analyze_stomp_running_compare.py` — amorçage ARM + fond scroll (`stomp_running_*`)

## Convention de nommage

Exemples : `3_scroll_HXEdit.json`, `01_connect_HXEdit.json`, `new_test_linux.json`.
