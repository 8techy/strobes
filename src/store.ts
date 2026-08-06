/**
 * Global UI state.
 *
 * Deliberately thin: the Rust side is the source of truth for connection,
 * engine and safety state, so this mostly caches the last known values and
 * exposes actions that refresh them.
 */

import { create } from "zustand";

import * as api from "./api";
import type {
  CatalogSummary,
  ConnectionStatus,
  Dtc,
  EcuScanResult,
  Effect,
  EngineStatus,
  LampInfo,
  LampVisualState,
  Preflight,
  Protocol,
  VehicleInfo,
} from "./types";

interface StrobeState {
  // Connection
  status: ConnectionStatus | null;
  vehicle: VehicleInfo | null;
  connecting: boolean;
  /** Last error, shown as a banner rather than thrown away. */
  error: string | null;

  // Diagnostics
  ecus: EcuScanResult[];
  dtcs: Dtc[];
  preflight: Preflight | null;

  // Catalog
  catalogs: CatalogSummary[];
  lamps: LampInfo[];

  // Effects
  effects: Effect[];
  engine: EngineStatus | null;
  /** Live lamp brightness, driven by engine step events. */
  lampVisuals: LampVisualState;

  simulatorAddress: string | null;

  setError: (error: string | null) => void;
  refreshStatus: () => Promise<void>;
  connect: (protocol: Protocol, host: string, port?: number) => Promise<void>;
  disconnect: () => Promise<void>;
  scanEcus: () => Promise<void>;
  loadDtcs: () => Promise<void>;
  runPreflight: () => Promise<void>;
  loadCatalogs: () => Promise<void>;
  chooseCatalog: (path: string) => Promise<void>;
  loadLamps: () => Promise<void>;
  loadEffects: () => Promise<void>;
  refreshEngine: () => Promise<void>;
  applyLampVisual: (lamp: number, level: number) => void;
  clearLampVisuals: () => void;
  startSimulator: (protocol: Protocol) => Promise<string>;
  stopSimulator: () => Promise<void>;
}

/** Runs an action, routing any failure into the error banner. */
async function guarded(action: () => Promise<void>, set: (error: string) => void) {
  try {
    await action();
  } catch (e) {
    set(e instanceof Error ? e.message : String(e));
  }
}

export const useStore = create<StrobeState>((set, get) => ({
  status: null,
  vehicle: null,
  connecting: false,
  error: null,
  ecus: [],
  dtcs: [],
  preflight: null,
  catalogs: [],
  lamps: [],
  effects: [],
  engine: null,
  lampVisuals: {},
  simulatorAddress: null,

  setError: (error) => set({ error }),

  refreshStatus: async () => {
    await guarded(
      async () => set({ status: await api.connectionStatus() }),
      (error) => set({ error }),
    );
  },

  connect: async (protocol, host, port) => {
    set({ connecting: true, error: null });
    try {
      const vehicle = await api.connectVehicle(protocol, host, port);
      set({ vehicle });
      await get().refreshStatus();
      await get().loadLamps();
    } catch (e) {
      set({ error: e instanceof Error ? e.message : String(e) });
    } finally {
      set({ connecting: false });
    }
  },

  disconnect: async () => {
    await guarded(async () => {
      await api.disconnectVehicle();
      // Clear everything that only makes sense for a live connection.
      set({
        vehicle: null,
        ecus: [],
        dtcs: [],
        preflight: null,
        engine: null,
        lampVisuals: {},
      });
      await get().refreshStatus();
    }, (error) => set({ error }));
  },

  scanEcus: async () => {
    await guarded(
      async () => set({ ecus: await api.scanEcus() }),
      (error) => set({ error }),
    );
  },

  loadDtcs: async () => {
    await guarded(
      async () => set({ dtcs: await api.readDtcs() }),
      (error) => set({ error }),
    );
  },

  runPreflight: async () => {
    await guarded(
      async () => set({ preflight: await api.runSafetyPreflight() }),
      (error) => set({ error }),
    );
  },

  loadCatalogs: async () => {
    await guarded(
      async () => set({ catalogs: await api.listCatalogs() }),
      (error) => set({ error }),
    );
  },

  chooseCatalog: async (path) => {
    await guarded(async () => {
      await api.loadCatalog(path);
      await get().refreshStatus();
      await get().loadLamps();
    }, (error) => set({ error }));
  },

  loadLamps: async () => {
    await guarded(
      async () => set({ lamps: await api.listLamps() }),
      (error) => set({ error }),
    );
  },

  loadEffects: async () => {
    await guarded(
      async () => set({ effects: await api.listEffects() }),
      (error) => set({ error }),
    );
  },

  refreshEngine: async () => {
    await guarded(
      async () => set({ engine: await api.engineStatus() }),
      (error) => set({ error }),
    );
  },

  applyLampVisual: (lamp, level) =>
    set((state) => ({ lampVisuals: { ...state.lampVisuals, [lamp]: level } })),

  clearLampVisuals: () => set({ lampVisuals: {} }),

  startSimulator: async (protocol) => {
    const address = await api.startSimulator(protocol);
    set({ simulatorAddress: address });
    await get().refreshStatus();
    return address;
  },

  stopSimulator: async () => {
    await guarded(async () => {
      await api.stopSimulator();
      set({ simulatorAddress: null });
      await get().refreshStatus();
    }, (error) => set({ error }));
  },
}));
