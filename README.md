strobe light management for f and g series over ENET (cable/wifi) </br>
got annoyed with bimmerlight being paid </br>
</br>
React UI (Vite + Tailwind)</br>
Tauri IPC</br>
Rust command scheduling (for timed activation)</br>
HSFZ and DoIP BMW protocols</br>
ZN8 / BRZ ISO-TP research spike</br>

<img width="639" height="339" alt="image" src="https://github.com/user-attachments/assets/2a246599-c1f1-462b-b035-a065650d5114" />


## Getting started

### Requirements

- Node 20+
- Rust stable (1.77+)
- Platform toolchain for Tauri 2: MSVC build tools and WebView2 on Windows,
  Xcode command line tools on macOS, `webkit2gtk` and `libayatana-appindicator` on
  Linux

### Run

```bash
npm install
npm run dev
```

Launches Tauri Desktop view

### Package

```bash
npm run build
```

Produces an MSI and NSIS installer on Windows, a `.dmg` on macOS, and `.deb` plus
AppImage on Linux, in `src-tauri/target/release/bundle/`.

## Architecture

```
src-tauri/
  src/                 Tauri shell: IPC surface and session state
  crates/
    slt-transport/     HSFZ, DoIP, and ISO-TP framing (CAN backend TBD)
    slt-uds/           UDS client, session keepalive, safety guard
    slt-catalog/       Chassis catalogs and the lamp enumeration
    slt-engine/        Scheduler, safety supervisor, effect library
    slt-sim/           Mock vehicle
src/
  routes/              Connect, Vehicle, Effects, Editor, Lab, Safety
  components/          Shell chrome and the lamp grid
  audio/               Beat detection
catalog/chassis/       Per-chassis TOML
```
