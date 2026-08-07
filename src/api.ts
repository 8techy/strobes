/**
 * Typed wrappers around the Tauri IPC commands.
 *
 * Every call funnels through here so the rest of the UI never touches `invoke`
 * directly, which keeps the command names in one place and gives each call a
 * real signature. Strobes is a desktop app only — there is no browser backend.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  CatalogSummary,
  ConnectionStatus,
  DiscoveredVehicle,
  Dtc,
  EcuScanResult,
  Effect,
  EngineEvent,
  EngineStatus,
  LampInfo,
  Preflight,
  Protocol,
  VehicleInfo,
} from "./types";

function assertDesktop(): void {
  if (!("__TAURI_INTERNALS__" in window)) {
    throw new Error(
      "Strobes only runs as the desktop app. Use npm run dev (or npm run tauri:dev).",
    );
  }
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  assertDesktop();
  return invoke<T>(command, args);
}

// -- Connection ------------------------------------------------------------

export const discoverVehicles = () => call<DiscoveredVehicle[]>("discover_vehicles");

export const connectVehicle = (protocol: Protocol, host: string, port?: number) =>
  call<VehicleInfo>("connect_vehicle", { protocol, host, port: port ?? null });

export const disconnectVehicle = () => call<void>("disconnect_vehicle");

export const connectionStatus = () => call<ConnectionStatus>("connection_status");

// -- Read-only diagnostics -------------------------------------------------

export const readVehicleInfo = () => call<VehicleInfo>("read_vehicle_info");

export const scanEcus = () => call<EcuScanResult[]>("scan_ecus");

export const readDtcs = (ecu?: number) => call<Dtc[]>("read_dtcs", { ecu: ecu ?? null });

export const clearDtcs = (ecu?: number) => call<void>("clear_dtcs", { ecu: ecu ?? null });

export const readShortCircuitCounters = () =>
  call<Record<string, number>>("read_short_circuit_counters");

export const runSafetyPreflight = () => call<Preflight>("run_safety_preflight");

// -- Catalog ---------------------------------------------------------------

export const listCatalogs = () => call<CatalogSummary[]>("list_catalogs");

export const loadCatalog = (path: string) => call<string>("load_catalog", { path });

export const listLamps = () => call<LampInfo[]>("list_lamps");

// -- Effects ---------------------------------------------------------------

export const listEffects = () => call<Effect[]>("list_effects");

export const setLamp = (lamp: number, level: number) =>
  call<void>("set_lamp", { lamp, level });

export const releaseAll = () => call<void>("release_all");

export const panicStop = () => call<void>("panic_stop");

export const startEffect = (effect: Effect) => call<void>("start_effect", { effect });

export const stopEffect = () => call<void>("stop_effect");

export const engineStatus = () => call<EngineStatus>("engine_status");

export const submitBeat = (bpm: number) => call<void>("submit_beat", { bpm });

export const setBeatSync = (enabled: boolean) => call<void>("set_beat_sync", { enabled });

// -- Simulator -------------------------------------------------------------

export const startSimulator = (protocol: Protocol) =>
  call<string>("start_simulator", { protocol });

export const stopSimulator = () => call<void>("stop_simulator");

export const simulatorStatus = () => call<string | null>("simulator_status");

// -- Events ----------------------------------------------------------------

/** Subscribes to engine events. Returns an unsubscribe function. */
export async function onEngineEvent(
  handler: (event: EngineEvent) => void,
): Promise<UnlistenFn> {
  assertDesktop();
  return listen<EngineEvent>("engine-event", (message) => handler(message.payload));
}
