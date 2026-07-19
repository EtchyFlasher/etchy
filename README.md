<div align="center">

# ⚡ Etchy

**The verified ISO → USB flasher.**

*Every byte written. Every byte checked. Every step visible.*

[![CI](https://img.shields.io/badge/CI-GitHub_Actions-2088FF?logo=githubactions&logoColor=white)](.github/workflows/ci.yml)
[![License: GPL-3.0](https://img.shields.io/badge/License-GPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/core-Rust-orange?logo=rust)](etchy-core)
[![Tauri](https://img.shields.io/badge/GUI-Tauri_2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
![Platforms](https://img.shields.io/badge/platforms-Linux%20·%20Windows%20·%20macOS-lightgrey)

</div>

---

## Why Etchy?

Most flashing tools have two problems: **you don't know if the write actually succeeded**, and **you stare at a bar that tells you nothing**. Etchy was built to fix exactly that:

| | |
|---|---|
| 🔒 **Mandatory verification** | After writing, Etchy reads **every byte back from the physical device** and compares SHA-256 hashes against the source. There is no skip button. If it says ✓, your USB *provably* matches the ISO. |
| 📊 **Real visual feedback** | Glowing progress ring, live MB/s speed graph, ETA, per-phase tracker (hash → write → sync → verify) — you always know what's happening and how long it'll take. |
| 🛡️ **Safe by design** | System disks are **never even listed**. Removable USB devices only, and you must type the drive's model name to unlock the flash button. |
| 🕵️ **Fake-drive detection** | Optional pre-check writes position-dependent patterns across the drive — catches counterfeit "128 GB" sticks and dying flash cells *before* you waste time flashing. |
| ✅ **Checksum validation** | Paste the official SHA-256 from the distro's website; Etchy refuses to write if the ISO doesn't match. |
| 💾 **Persistence (optional)** | One checkbox adds a `casper-rw` / `persistence` partition for Ubuntu/Debian live USBs (Linux host only). |
| 🌍 **Themes & languages** | Dark/light theme, English/Español/Deutsch/Français out of the box — translations welcome! |

## Screenshot

> Dark, techy, glowing. Run the UI demo (below) to see it live.

## Install

Grab the latest build from **[Releases](../../releases)**:

| Platform | Package |
|---|---|
| Debian/Ubuntu | `etchy_x.y.z_amd64.deb` — `sudo apt install ./etchy_*.deb` |
| Any Linux | `etchy_x.y.z_amd64.AppImage` — `chmod +x` and run |
| Windows 10/11 | `etchy_x.y.z_x64-setup.exe` |
| macOS | `Etchy_x.y.z_universal.dmg` |

## How it works

```
┌─────────────┐   Tauri IPC    ┌──────────────┐   pkexec / UAC /   ┌──────────────┐
│  GUI (web    │ ─────────────▶ │  etchy (app)  │   osascript        │ etchy-helper  │
│  tech, no    │ ◀───────────── │  unprivileged │ ─────────────────▶ │ root/admin,   │
│  privileges) │  JSON events   │               │  JSON over pipes   │ one job only  │
└─────────────┘                └──────────────┘                    └──────┬───────┘
                                                                          │
                                                                   ┌──────▼───────┐
                                                                   │  etchy-core   │
                                                                   │ write→sync→   │
                                                                   │ VERIFY engine │
                                                                   └──────────────┘
```

- The GUI **never runs as root/admin**. Only the tiny `etchy-helper` process gets elevated, per job.
- The flash pipeline: `hash source → (bad-block check) → write (4 MiB chunks, sector-aligned) → flush & sync → read back & verify → (persistence)`.
- Device enumeration is the **single safety gatekeeper** — on all three OSes only removable USB, non-system disks are ever returned.

## Building from source

**Prerequisites (Debian/Ubuntu):**

```bash
sudo apt update
sudo apt install -y build-essential curl libwebkit2gtk-4.1-dev \
  libgtk-3-dev libappindicator3-dev librsvg2-dev patchelf
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # Rust
cargo install tauri-cli --version "^2" --locked
```

**Build & run:**

```bash
git clone https://github.com/EtchyFlasher/etchy && cd etchy
cargo build --release -p etchy-helper     # the privileged worker
cargo tauri dev                            # run the app (dev)
cargo tauri build                          # produce .deb / .AppImage / installer
```

**Run the engine's tests:**

```bash
cargo test -p etchy-core
```

**Preview the GUI without hardware** (demo mode, mock backend — safe anywhere):

```bash
cd ui && python3 -m http.server 3000
# open http://localhost:3000 — the "DEMO MODE" badge confirms no device access
```

## Repository layout

```
etchy-core/     # engine: device enumeration, flash pipeline, verify, badblocks, persistence
  src/platform/ #   linux.rs (sysfs), windows.rs (CIM + volume locking), macos.rs (diskutil)
etchy-helper/   # tiny privileged CLI wrapper (JSON progress over stdout, cancel over stdin)
src-tauri/      # Tauri 2 app: IPC commands, elevation (pkexec/UAC/osascript), bundling
ui/             # frontend: vanilla JS + CSS, i18n, demo-mode mock backend
.github/        # CI (test+clippy on 3 OSes) and tag-triggered release builds
```

## Safety model

1. **Enumeration filter** — `removable == true`, bus == USB, never the disk backing `/` (Linux), `IsSystem/IsBoot == false` (Windows), `Internal == false` (macOS).
2. **Preflight re-check** — the flash pipeline re-enumerates and refuses any device that isn't in the eligible list *at flash time*.
3. **Type-to-confirm** — the flash button stays locked until you type the target drive's model name.
4. **Exclusive access** — `O_EXCL` (Linux) / `FSCTL_LOCK_VOLUME` (Windows) / `unmountDisk force` (macOS) before a single byte is written.
5. **Mandatory verify** — the report shows both hashes; they must match or the job fails loudly.

## Enabling CI (one-time)

The GitHub Actions workflows live in [`ci-workflows/`](ci-workflows) because the deployment
bot can't push workflow files. To activate CI + automated releases, move them once via the
GitHub web UI or a local clone:

```bash
mkdir -p .github/workflows && git mv ci-workflows/*.yml .github/workflows/ && git commit -m "Enable CI" && git push
```

After that, pushing a tag like `v0.1.0` auto-builds the Windows installer, macOS dmg,
.deb and AppImage as a draft release.

## Contributing

PRs welcome! Easy first contributions:
- 🌍 add a language: copy the `en` block in `ui/i18n/strings.js`
- 🧪 add engine tests
- 🐛 file bugs with the report shown on the error screen

## License

[GPL-3.0-or-later](LICENSE) — free forever, forks stay free.
