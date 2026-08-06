# Strobelight

A desktop application that connects to BMW F/G-series vehicles over an ENET cable
and choreographs the exterior lighting modules into programmable light shows,
optionally synced to music.

Rust handles the diagnostic transports and the millisecond-accurate scheduler;
React handles the interface.

> **Not affiliated with BMW AG.** Contains no BMW source code, no SGBD files and
> no data extracted from BMW software. See [Legal](#legal).

---

## Read this first

Strobelight actuates the lighting outputs on your car's body controller. That is
inherently lower-risk than coding or flashing, because nothing is written to
non-volatile memory and the vehicle reverts on its own when the diagnostic
session lapses. It is not zero-risk.

- **Private property only.** Flashing exterior lights on public roads is illegal
  in most jurisdictions, and patterns resembling emergency vehicles are
  specifically prohibited.
- **Ignition on, engine off, car stationary.** The module enforces this itself and
  will refuse with a reason if the conditions are wrong.
- **Connect a battery charger.** Low supply voltage during actuation is the
  best-documented cause of module faults.
- **Run the preflight check.** Your body controller counts short-circuit
  shutdowns per output. When a counter reaches its limit that output is
  **permanently disabled** and the module must be replaced. Strobelight reads
  these counters and refuses to drive any output that already has faults
  recorded, but it can only do that if your catalog defines the counter
  identifier.

The Safety screen documents what the application refuses to do, and those refusals
are enforced in the lowest layer that understands UDS, so no catalog entry, effect
or UI path can bypass them.

---

## Hardware

Your car has a 16-pin J1962 OBD-II port. There is no USB-C port on the vehicle
side — what varies is the adapter at your laptop end.

Strobelight uses **ENET**, which is Ethernet-over-OBD, because it is by far the
fastest option and effect steps are latency-sensitive. On a modern laptop that
means:

```
laptop ──USB-C Ethernet adapter── RJ45 ──ENET cable── OBD-II port
```

The cable must bridge **OBD pin 8 to pin 16 through a 510 ohm resistor**. Pin 8 is
the Ethernet activation line; without it the car's Ethernet PHY stays powered down
entirely and nothing responds at any level. Commercial ENET cables already include
the resistor. This is the most common reason a home-made cable appears dead.

### Network setup

The gateway uses link-local addressing (`169.254.x.x`) and announces itself over
UDP, so **you should not need to configure anything** — press "Scan the network"
and Strobelight probes both protocols concurrently.

If discovery finds nothing:

1. Confirm the adapter shows a link. On Windows, `ipconfig` should list the
   interface with a `169.254.x.x` address.
2. Set the interface to DHCP, not a static address. Link-local self-assignment is
   what you want.
3. Disable other interfaces temporarily. A broadcast on the wrong adapter is a
   frequent cause of silent discovery failure.
4. Try the address directly. F-series gateways commonly answer on
   `169.254.87.130`.

### Which protocol

| Chassis                | Transport | Port  |
| ---------------------- | --------- | ----- |
| F-series (F20-F87)     | HSFZ      | 6801  |
| G-series, U-series     | DoIP      | 13400 |

You do not need to know which: discovery probes both. HSFZ is BMW-proprietary and
needs no handshake; DoIP is ISO 13400 and requires routing activation, which
Strobelight performs automatically.

---

## No car? Use the simulator

The built-in simulator serves real HSFZ and DoIP traffic from mock ECUs, so the
entire application works without a vehicle. On the Connect screen, start it as
either chassis type, then load the `SIM` catalog.

It deliberately reproduces awkward real behaviours — gateway acknowledgement
frames, `responsePending`, session timeouts dropping actuation, and refusals for a
running engine or low voltage — so those paths are exercised somewhere other than
someone's car.

---

## Catalogs, and why effects are disabled at first

The lamp *numbering* is public and built in: `LAMPNRTEXTE` gives
`0x30 = TMS_LEUCHTRING_L` and around sixty others, identically across `FEM_20`,
`BDC`, `BDC_G05` and `BDC_G11`.

What is **not** public is the data identifier that controls a lamp. That lives in
the `SG_Funktionen` table inside proprietary SGBD `.PRG` files shipped with ISTA
and E-Sys. It is BMW copyright and cannot be redistributed, so Strobelight ships
none of it.

Consequently `f-series.toml` and `g-series.toml` are **templates with placeholder
identifiers** marked `verified = false`, and the safety supervisor refuses to
transmit them. Filling them in from your own licensed BMW software is documented
in [catalog/README.md](catalog/README.md); the first three steps need no
connection to the car at all.

Everything read-only works immediately without any of this: connect, scan modules,
read the VIN, read fault codes.

---

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
npm run tauri:dev
```

### Test

```bash
cd src-tauri
cargo test --workspace
```

### Package

```bash
npm run tauri:build
```

Produces an MSI and NSIS installer on Windows, a `.dmg` on macOS, and `.deb` plus
AppImage on Linux, in `src-tauri/target/release/bundle/`.

### Regenerate the app icon

The icon is produced by a script rather than committed as opaque binary:

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

Two decisions worth knowing about:

**The scheduler is in Rust, not JavaScript.** Effect steps can be as short as
20 ms, and `setTimeout` jitter of 5-15 ms would be a visible fraction of a step.
Beat detection runs in the frontend because that is where Web Audio lives, but it
only emits beat *events* — timing decisions stay in Rust. Step deadlines are
computed from a fixed origin rather than "now plus duration", so error cannot
accumulate over a long show.

**Vehicle specifics are data, not code.** Adding a chassis means writing a TOML
file. The schema mirrors BMW's own `SG_Funktionen`, including the scaling and
byte-order columns, because a parameter value is not always a raw byte.

### Performance ceiling

ENET round trips are roughly 1-3 ms, so the transport is not the bottleneck. The
LIN bus between the body controller and the TMS/LHM headlight modules updates on
the order of tens of milliseconds, putting the practical floor around **20-50 ms
per lamp change**. The editor clamps step durations to that floor rather than
letting you author something the car silently fails to render.

---

## Legal

The protocol details Strobelight is built on come from public standards
(ISO 14229, ISO 13400), BMW's own published tooling documentation, open-source
implementations, and public community research. No BMW code or data files are
included or redistributed.

Catalog files containing vehicle-specific identifiers are authored by users from
software they license, and are not distributed with the application.

You are responsible for how you use this. Do not use it on public roads.
