# Catalogs

A catalog tells Strobes how to turn "switch on the left headlight ring" into
the exact UDS bytes your car expects. Everything vehicle-specific lives here, so
adding support for a chassis means writing a TOML file rather than changing code.

## Platforms

| Catalog | Transport | Notes |
|---------|-----------|-------|
| `f-series.toml` | HSFZ (BMW ENET) | Template, unverified identifiers |
| `g-series.toml` | DoIP (BMW ENET) | Template, unverified identifiers |
| `simulator.toml` | HSFZ | Fully verified against the built-in mock |
| `zn8.toml` | ISO-TP (CAN) | Research spike for GR86/BRZ — see [zn8-research.md](zn8-research.md) |

BMW chassis still share one lamp numbering table. ZN8 uses separate `ZN8_*`
codes; the ISO-TP SocketCAN backend is not wired yet, so connecting with the
ZN8 catalog will fail until that lands.

## Why the shipped catalogs are incomplete

The lamp *numbering* is public and already built in: `LAMPNRTEXTE` gives
`0x30 = TMS_LEUCHTRING_L` and about sixty others, and that table is identical
across `FEM_20`, `BDC`, `BDC_G05` and `BDC_G11`.

What is not public is the **data identifier** that controls a lamp. That lives in
the `SG_Funktionen` table inside proprietary SGBD `.PRG` files shipped with ISTA
and E-Sys. It is BMW copyright and cannot be redistributed, so Strobes ships
none of it. `f-series.toml` and `g-series.toml` are templates with placeholder
identifiers and `verified = false`, and the safety supervisor refuses to transmit
an unverified action unless you explicitly turn on research mode.

`simulator.toml` is the exception: it is fully verified because its identifier is
one the built-in simulator invents. Use it to explore the whole application
without a car.

## Filling in a template

The procedure is in [docs/protocol-research.md](../docs/protocol-research.md)
section 6. In brief, and note that steps 1 to 3 need no connection to the car:

1. Open the SGBD for your chassis in Tool32 (`FEM_20.PRG` for F-series,
   `BDC_G05.PRG` or `BDC_G11.PRG` for G-series).
2. Open the `SG_Funktionen` table from the Tabellen-Info window.
3. Find rows whose `INFO` mentions lighting and whose `SERVICE` is `2F` or `31`.
   The `ID` column is the identifier you need.
4. Copy the `ARG_TABELLE` parameter definitions into `[[action.param]]` blocks,
   preserving `DATENTYP`, `L/H`, `MUL`, `DIV` and `ADD`. These matter: a value is
   not always a raw byte.
5. Confirm on your own car, then set `verified = true`.

If reading the table is not an option, capture instead: HSFZ and DoIP are
plaintext TCP, so running a lighting `steuern_*` job in Tool32 with Wireshark
recording the ENET interface shows you the bytes directly.

For ZN8 / ZC8, capture Techstream or SSM5 lighting active tests over OBD-II
CAN instead. Details: [zn8-research.md](zn8-research.md).

## Schema

```toml
schema_version = 1

[chassis]
id = "F3x"                  # short identifier
name = "BMW F-series"       # shown in the UI
transport = "hsfz"          # "hsfz", "doip", or "isotp"
notes = ""

[[ecu]]
name = "FEM_BODY"           # referenced by actions
address = 0x40              # diagnostic address

[[action]]
id = "lamp.set"             # "lamp.set" is required for lamp control
ecu = "FEM_BODY"
service = 0x2F              # 0x2F (IOControl) or 0x31 (RoutineControl)
identifier = 0xD000         # the DID or RID
session = 0x03              # extended session
control_actuate = 0x03      # STA for 0x2F, STR for 0x31
control_release = 0x00      # RCTECU for 0x2F, STPR for 0x31
min_dwell_ms = 40           # minimum time between changes to one lamp
verified = false

  [[action.param]]
  name = "lamp"             # "lamp" and "level" are the names the engine looks for
  datatype = "char"         # char (1 byte), int (2), long (4)
  byte_order = "high"       # "high" or "low", from the L/H column
  mul = 1.0                 # physical = raw * mul / div + add
  div = 1.0
  add = 0.0
  min = 0.0
  max = 100.0

[lamps]
available = ["TFL_L", "TFL_R"]   # omit or leave empty to offer every BMW lamp
```

### Required action ids

- `lamp.set` — controls one lamp. Must have a `lamp` parameter; a `level`
  parameter is optional and omitted for on/off-only modules. Without this action
  the engine will not start.
- `lamp.counters` — optional. Reads per-lamp short-circuit counters so the
  preflight check can refuse to drive an already-faulty output. Strongly
  recommended: this is the check that protects against permanently disabling a
  lamp driver.

### Request byte order

The two services lay their bytes out differently, and Strobes builds each
correctly from `service`:

- `0x2F`: `2F <identifier_hi> <identifier_lo> <control> <params...>`
- `0x31`: `31 <control> <identifier_hi> <identifier_lo> <params...>`

## Contributing

Verified catalogs for real chassis are welcome, but please only submit
identifiers you confirmed on your own vehicle, and do not paste in BMW's SGBD
files or their contents wholesale. A catalog naming the handful of identifiers
you tested is fine; a dump of `SG_Funktionen` is not.
