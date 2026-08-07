strobe light management for f and g series over ENET (cable/wifi) </br>
got annoyed with bimmerlight being paid </br>
</br>
React UI (Vite + Tailwind)</br>
Tauri IPC</br>
Rust command scheduling (for timed activation)</br>
HSFZ and DoIP BMW protocols</br>


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

That launches the Tauri desktop window (Vite only runs as the webview UI
bundler in the background — there is no standalone browser app).

### Test

```bash
cd src-tauri
cargo test --workspace
```

### Package

```bash
npm run build
```

Produces an MSI and NSIS installer on Windows, a `.dmg` on macOS, and `.deb` plus
AppImage on Linux, in `src-tauri/target/release/bundle/`.

### Regenerate the app icon

Brand marks live in `public/`. Sync the dark icon into Tauri's source slot, then
expand it to every platform size:

```bash
node scripts/generate-icon.mjs
npx tauri icon src-tauri/icons/source.png
```

---

## Architecture

```
src-tauri/
  src/                 Tauri shell: IPC surface and session state
  crates/
    slt-transport/     HSFZ and DoIP framing, UDP discovery
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
