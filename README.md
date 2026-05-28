# Open Island Windows

<p align="center">
  <strong>Windows Dynamic Island for AI Coding Agents</strong>
</p>

<p align="center">
  Monitor sessions, approve permissions, and jump to terminal — all from a floating island at the top of your screen.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0-green" alt="GPL-3.0"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%2F11-blue" alt="Platform">
  <img src="https://img.shields.io/badge/built%20with-Tauri%202%20%2B%20Rust-orange" alt="Tauri + Rust">
</p>

---

## Features

- **Session Monitoring** — See all active AI coding sessions in a compact floating pill
- **Permission Approval** — Allow, Deny, or Always-allow tool permissions directly from the island
- **Question Display** — View agent questions with selectable options
- **Terminal Jump** — One-click to jump back to the terminal running your AI agent
- **Pixel Pet** — Animated pixel-art companion showing session status
- **Sound Notifications** — Audio alerts when agents need attention
- **Auto-approve Mode** — Skip approval for trusted workflows
- **Multi-display Support** — Works across multiple monitors
- **I18N** — Chinese and English interface

## Supported Agents

| Agent | Status |
|-------|--------|
| Claude Code | ✅ Full support |
| Codex | ✅ Full support |
| Gemini CLI | ✅ Supported |
| Cursor | ✅ Supported |
| OpenCode | ✅ Supported |
| Qwen Code | ✅ Supported |
| Kimi CLI | ✅ Supported |
| Qoder | ✅ Supported |
| Factory | ✅ Supported |
| CodeBuddy | ✅ Supported |

## Architecture

`
┌─────────────────────────────────────────────┐
│  React Frontend (TypeScript)                │
│  ├── Dynamic Island UI (CSS transitions)    │
│  ├── Session cards & approval flows         │
│  └── Settings & configuration               │
├─────────────────────────────────────────────┤
│  Tauri Bridge                               │
│  ├── IPC commands                           │
│  └── Event system                           │
├─────────────────────────────────────────────┤
│  Rust Backend                               │
│  ├── HTTP hooks server (port 51515)         │
│  ├── TCP bridge server (port 19841)         │
│  ├── Process monitor                        │
│  ├── Session store                          │
│  └── Native Win32 window (rounded region)   │
└─────────────────────────────────────────────┘
`

## Install

### Download

Download the latest release from [GitHub Releases](#):

- Open Island_0.1.0_x64-setup.exe — NSIS installer (recommended)
- Open Island_0.1.0_x64_en-US.msi — MSI installer

### Build from Source

`ash
# Prerequisites: Node.js >= 18, Rust, Tauri CLI
npm install
npm run tauri build
`

Output: D:\open-island-build-target\release\open-island.exe

## Usage

1. Launch Open Island — a compact pill appears at the top of your screen
2. Click the pill to expand and see active sessions
3. When an agent needs permission, the island auto-expands to show the approval card
4. Click Allow/Deny/Always to respond
5. Click any session to jump back to its terminal

### CLI Hooks

Open Island automatically installs HTTP hooks for Claude Code. For other agents, hooks are installed via the Settings panel.

## Credits

This project was inspired by the growing ecosystem of AI agent monitoring tools:

- **[open-vibe-island](https://github.com/Octane0411/open-vibe-island)** — The original macOS Dynamic Island for AI coding agents (GPL-3.0), which served as a reference for the concept and architecture
- **[vibeisland.app](https://vibeisland.app)** — The commercial macOS product that pioneered the Dynamic Island UI pattern for AI agents

The Windows port involved significant architectural changes:
- Rewrote the entire UI layer in React + TypeScript (original: Swift/SwiftUI)
- Implemented native Win32 window management (rounded regions, no-activate mode)
- Built a custom HTTP hooks server for Claude Code integration
- Added process monitoring via Win32 API
- Designed an original pixel-art pet (owl character)

## License

This project is licensed under the [GNU General Public License v3.0](LICENSE).

Copyright (C) 2025 Open Island Contributors

This program is free software: you can redistribute it and/or modify it under the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.

You should have received a copy of the GNU General Public License along with this program. If not, see <https://www.gnu.org/licenses/>.
