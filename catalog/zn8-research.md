# ZN8 / ZC8 research notes (Toyota GR86 / Subaru BRZ)

Status: **spike**. Strobes today is BMW ENET (HSFZ / DoIP). This document
captures what we know about adding the 2022+ GR86 (ZN8) and BRZ (ZC8), and what
landed in the first code spike.

Chassis codes:

| Market name | Chassis | Platform |
|-------------|---------|----------|
| Toyota GR86 | ZN8 | FA24, shared with ZC8 |
| Subaru BRZ | ZC8 | same generation as ZN8 |

## What is different from BMW

| Concern | BMW F/G (current) | ZN8 / ZC8 |
|---------|-------------------|-----------|
| Physical link | Ethernet (ENET cable / Wi‑Fi) | CAN at 500 kbit/s via OBD-II |
| Wire protocol | HSFZ (F) or DoIP (G) | ISO-TP (ISO 15765-2) over CAN |
| Discovery | UDP vehicle ID / HSFZ probe | Scan known diagnostic IDs on the bus |
| Body controller | FEM / BDC | BCM (Body Control Module) |
| Lamp index | Public `LAMPNRTEXTE` | Not published; OEM-specific |
| Actuation | UDS `0x2F` / `0x31` (catalog DIDs) | Likely UDS `0x2F` or `0x31`, often behind `0x27` security access |
| Adapter | ENET / DoIP gateway | USB-CAN / SocketCAN / ELM327-class ISO-TP |

A chassis TOML alone is **not** enough. Transport and ECU addressing must change.

## Buses on the car

Community and aftermarket tooling (AiM, Racelogic, Dauntless, forum reports)
consistently describe multiple CAN buses:

1. **OBD-II diagnostic CAN** — standard OBD Mode 01/22 plus enhanced UDS to
   modules. This is the correct bus for diagnostics and bi-directional tests.
2. **Body / ASC CAN** — raw traffic useful for logging and some body features.
   Standard diagnostic sessions are **not** available here; OBD requests on ASC
   do not get responses.

For Strobes, target the **OBD-II port** first. ASC adapters are a later option
for broadcast-only experiments, not for UDS actuation.

## Working hypotheses (unverified)

Treat every identifier below as a research lead until confirmed on a real car
with a capture. Do not ship `verified = true` without that confirmation.

### Diagnostic addressing

Common community reports for 2022+ BRZ / GR86 style UDS:

| Role | Request ID | Response ID | Notes |
|------|------------|-------------|-------|
| ECM (powertrain) | `0x7E0` | `0x7E8` | Never an actuation target for Strobes |
| BCM (body) | `0x7E1` | `0x7E9` | Primary lighting / body candidate |

Other IDs may exist behind a gateway. A scan of `0x7E0`–`0x7EF` (and any
Toyota/Subaru gateway scheme) should be the first live test.

### Security access

Body actuators on this platform are widely reported to require UDS service
`0x27` (SecurityAccess) before `0x2F` / `0x31` succeed. Third-party writeups
quote seed/key algorithms; **none of those claims are treated as fact here**.
Capturing Techstream / SSM5 active tests with Wireshark or a CAN logger is the
right way to learn:

1. Whether security access is required for lighting IO control
2. Which security level
3. Which DIDs / RIDs actually drive lamps

Strobes already refuses persistent writes (`0x2E`) and programming session.
Any ZN8 path must keep those guards and add an explicit, reviewable security
access path before actuation.

### Lighting control surface

Aftermarket products for GR86/BRZ expose settings such as DRL enable and
“pace car” DRL flash with hazards. That proves the body network can influence
exterior lamps, but not that a free UDS IO-control DID exists for per-lamp
strobe timing.

Candidate research path:

1. Run Techstream / SSM5 **Active Test** for each exterior lamp while capturing
   ISO-TP on OBD-II.
2. Note session (`0x10`), security (`0x27`), and actuation (`0x2F` or `0x31`)
   bytes.
3. Fill `catalog/chassis/zn8.toml` identifiers and mark `verified = true` only
   after a successful own-car confirmation.
4. Measure how short a dwell the BCM tolerates before lamp monitoring faults.

Decorative / low-risk first targets: DRLs, parking lamps, side markers.
Keep turn signals, brake, and reverse gated as safety-critical (already a
Strobes concept).

## Spike contents (this branch)

Shipped as research scaffolding, not production vehicle support:

1. **`slt-transport` ISO-TP framing** (`isotp` module) — single-frame, first /
   consecutive frame, and flow-control encode/decode with unit tests. No
   SocketCAN dependency yet.
2. **`Protocol::IsoTp`** — catalog and UI can name the transport. Opening a live
   IsoTp connection returns a clear “adapter not wired” error until a CAN
   backend lands.
3. **`catalog/chassis/zn8.toml`** — template with hypothesized BCM address and
   placeholder lamp action (`verified = false`). Research mode required to
   transmit.
4. **ZN8 lamp codes** in the catalog lamp table (`ZN8_*`) so the UI can list a
   ZN8-shaped set without pretending BMW `LAMPNR` values apply.
5. **This document** and catalog README pointers.

## Suggested implementation order

1. **SocketCAN (or USB-CAN) backend** behind `Protocol::IsoTp`
   - Host string = interface name (`can0`, `vcan0`)
   - Pair request/response IDs per ECU from the catalog
2. **In-memory / `vcan` simulator** so CI can exercise ISO-TP without a car
3. **Catalog-driven ECU scan** (stop hardcoding BMW FEM/BDC addresses for ZN8)
4. **SecurityAccess helper** in `slt-uds` (seed request + key send), still
   blocked from inventing OEM algorithms in-tree
5. **Live capture session** against a ZN8/ZC8 to fill real DIDs
6. **Effects / dwell tuning** once one lamp actuates cleanly

## Hardware for the next step

Minimum useful lab setup:

- 2022+ GR86 or BRZ
- USB-CAN adapter with ISO-TP support (SocketCAN on Linux is ideal)
- Optional: Techstream or SSM5 for ground-truth active tests
- CAN logger (SavvyCAN, Wireshark with SocketCAN, or candump)

Until that hardware exists in the loop, keep identifiers unverified and refuse
transmission outside research mode.

## References (external, not endorsements)

- ISO 15765-2 (ISO-TP) and ISO 14229 (UDS)
- AiM / Racelogic ZN8 CAN channel docs (broadcast logging, not UDS actuation)
- Community notes on OBD vs ASC port behaviour for GR86/BRZ
- Aftermarket bi-directional tools for GR86/BRZ (evidence that enhanced module
  access exists over OBD; proprietary mapping not reusable here)
