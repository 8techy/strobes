/**
 * Lab screen: drive one lamp at a time.
 *
 * The most useful screen when verifying a freshly filled-in catalog, because it
 * isolates a single request. It reports the exact negative response the module
 * gave rather than a generic failure, since that is the information you need to
 * work out whether an identifier is wrong.
 */

import { useEffect, useState } from "react";

import * as api from "../api";
import { LampGrid } from "../components/LampGrid";
import { useStore } from "../store";
import type { LampInfo } from "../types";

export function Lab() {
  const lamps = useStore((s) => s.lamps);
  const loadLamps = useStore((s) => s.loadLamps);
  const lampVisuals = useStore((s) => s.lampVisuals);
  const applyLampVisual = useStore((s) => s.applyLampVisual);
  const clearLampVisuals = useStore((s) => s.clearLampVisuals);
  const preflight = useStore((s) => s.preflight);
  const engineReady = useStore((s) => s.status?.engineReady ?? false);
  const engine = useStore((s) => s.engine);

  const [level, setLevel] = useState(100);
  const [showRegulated, setShowRegulated] = useState(false);
  const [lastResult, setLastResult] = useState<string | null>(null);

  useEffect(() => {
    void loadLamps();
  }, [loadLamps]);

  const degraded = new Set(preflight?.degraded_lamps.map((l) => l.lamp) ?? []);

  async function toggle(lamp: LampInfo) {
    const current = lampVisuals[lamp.id] ?? 0;
    const next = current > 0 ? 0 : level;
    try {
      await api.setLamp(lamp.id, next);
      applyLampVisual(lamp.id, next);
      setLastResult(
        `${lamp.code} set to ${next}% — module accepted the request.`,
      );
    } catch (e) {
      setLastResult(e instanceof Error ? e.message : String(e));
    }
  }

  async function release() {
    try {
      await api.releaseAll();
      clearLampVisuals();
      setLastResult("All outputs handed back to the car.");
    } catch (e) {
      setLastResult(e instanceof Error ? e.message : String(e));
    }
  }

  if (!engineReady) {
    return (
      <p className="text-sm text-[var(--color-ink-400)]">
        Connect to a vehicle and load a catalog to drive lamps.
      </p>
    );
  }

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      <div className="card flex flex-wrap items-end gap-4 p-4">
        <div className="min-w-48 flex-1">
          <label className="label" htmlFor="level">
            Brightness {level}%
          </label>
          <input
            id="level"
            type="range"
            min={0}
            max={100}
            value={level}
            onChange={(e) => setLevel(Number(e.target.value))}
            className="w-full"
          />
        </div>

        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={showRegulated}
            onChange={(e) => setShowRegulated(e.target.checked)}
          />
          Show regulated signalling devices
        </label>

        <button className="btn btn-ghost" onClick={() => void release()}>
          Release all
        </button>
      </div>

      {engine?.running && (
        <div
          className="card p-3 text-sm"
          style={{ borderColor: "var(--color-amber-glow)" }}
        >
          An effect is running. Stop it before driving lamps by hand.
        </div>
      )}

      {lastResult && (
        <div className="card p-3 text-sm text-[var(--color-ink-300)]">{lastResult}</div>
      )}

      <div className="card p-4">
        <h2 className="mb-1 text-base font-bold">Outputs</h2>
        <p className="mb-4 text-xs text-[var(--color-ink-400)]">
          Click to toggle. Minimum {engine?.minDwellMs ?? 40} ms between changes to
          the same output.
        </p>
        <LampGrid
          lamps={lamps}
          levels={lampVisuals}
          degraded={degraded}
          onPick={(lamp) => void toggle(lamp)}
          featuredOnly={!showRegulated}
        />
      </div>
    </div>
  );
}
