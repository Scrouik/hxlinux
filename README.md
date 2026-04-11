# HXLinux

Open source HX Stomp XL editor for Linux - Built with Tauri (Rust + TypeScript)

## Status

🚧 **Work in progress** 🚧

This project is in early development. Current state:

### Working
- ✅ Native USB connection to HX Stomp XL under Linux
- ✅ Complete USB handshake protocol
- ✅ Reading 124 preset names from device
- ✅ Graphical interface with preset list

### In progress
- 🔄 Correct preset bank ordering (01A, 01B...)
- 🔄 Reading preset parameters
- 🔄 Real-time parameter editing

### Planned
- 📋 Signal chain visualization
- 📋 Parameter sliders (auto-generated from Line 6 .models files)
- 📋 AI-powered preset generation by prompt
- 📋 Export/import .hlx files

## Requirements

- Linux (Ubuntu/Debian)
- HX Stomp XL connected via USB
- HX Edit installed (Windows or Mac) for .models files

## Credits

USB protocol reverse engineering inspired by 
[kempline/helix_usb](https://github.com/kempline/helix_usb)
