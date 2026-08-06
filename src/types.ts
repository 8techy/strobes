/**
 * Types mirroring the Rust IPC surface in `src-tauri/src/ipc.rs`.
 *
 * Kept hand-written rather than generated so the shapes stay readable, but they
 * must be updated alongside the Rust structs.
 */

export type Protocol = "hsfz" | "doip";

export interface DiscoveredVehicle {
  ip: string;
  port: number;
  protocol: Protocol;
  vin: string | null;
  logical_address: number | null;
}

export interface VehicleInfo {
  vin: string | null;
  protocol: string;
  gateway_serial: string | null;
}

export interface ConnectionStatus {
  connected: boolean;
  protocol: string | null;
  host: string | null;
  catalogId: string | null;
  catalogVerified: boolean;
  engineReady: boolean;
  simulatorRunning: boolean;
}

export interface EcuScanResult {
  address: number;
  address_hex: string;
  label: string;
  present: boolean;
  serial: string | null;
  note: string | null;
}

export interface Dtc {
  code: number;
  code_hex: string;
  status: number;
  confirmed: boolean;
  pending: boolean;
  warning_indicator: boolean;
}

export interface DegradedLamp {
  lamp: number;
  code: string;
  name: string;
  counter: number;
  locked_out: boolean;
}

export interface Preflight {
  passed: boolean;
  dtcs_before: Dtc[];
  degraded_lamps: DegradedLamp[];
  blockers: string[];
  warnings: string[];
  catalog_verified: boolean;
}

export interface LampInfo {
  id: number;
  idHex: string;
  code: string;
  name: string;
  group: string;
  safetyCritical: boolean;
  featured: boolean;
}

export interface CatalogSummary {
  path: string;
  chassisId: string;
  name: string;
  transport: string;
  actionCount: number;
  verified: boolean;
  notes: string;
}

export interface LampCommand {
  lamp: number;
  level: number;
}

export interface Step {
  commands: LampCommand[];
  duration_ms: number;
}

export type Timing = "fixed" | "perBeat";

export interface Effect {
  id: string;
  name: string;
  description: string;
  steps: Step[];
  looping: boolean;
  timing: Timing;
}

export interface EngineStatus {
  running: boolean;
  effectId: string | null;
  stepIndex: number;
  stepCount: number;
  heldLamps: number[];
  bpm: number | null;
  beatSync: boolean;
  minDwellMs: number;
  researchMode: boolean;
}

export type EngineEvent =
  | { type: "started"; effect_id: string }
  | { type: "step"; effect_id: string; index: number; commands: LampCommand[] }
  | { type: "stopped"; effect_id: string; reason: string }
  | { type: "released"; lamps: number[] }
  | { type: "error"; message: string }
  | { type: "status"; [key: string]: unknown };

/** A lamp's live on-screen state, driven by engine step events. */
export interface LampVisualState {
  [lampId: number]: number;
}
