/**
 * Effects screen: pick a preset, run it, and optionally sync it to music.
 */

import { useEffect, useRef, useState } from "react";

import * as api from "../api";
import { BeatDetector } from "../audio/beat";
import { LampGrid } from "../components/LampGrid";
import { useStore } from "../store";
import type { Effect } from "../types";

/** Below this the car's LIN bus to the headlight modules cannot keep up. */
const RENDERABLE_STEP_MS = 20;

function BeatSync() {
  const detector = useRef<BeatDetector | null>(null);
  const beatErrorReported = useRef(false);
  const [active, setActive] = useState(false);
  const [level, setLevel] = useState(0);
  const [bpm, setBpm] = useState<number | null>(null);
  const setError = useStore((s) => s.setError);

  // Tear down the audio stream if the screen unmounts while listening.
  useEffect(() => {
    return () => {
      detector.current?.stop();
      detector.current = null;
    };
  }, []);

  async function toggle() {
    if (active) {
      detector.current?.stop();
      detector.current = null;
      beatErrorReported.current = false;
      setActive(false);
      setLevel(0);
      setBpm(null);
      try {
        await api.setBeatSync(false);
      } catch {
        // Losing the engine while stopping is not worth reporting.
      }
      return;
    }

    try {
      const instance = new BeatDetector({
        onBeat: (detected) => {
          setBpm(detected > 0 ? detected : null);
          void api.submitBeat(detected).catch((e) => {
            if (beatErrorReported.current) return;
            beatErrorReported.current = true;
            setError(
              e instanceof Error
                ? `Beat sync failed: ${e.message}`
                : String(e),
            );
          });
        },
        onLevel: setLevel,
      });
      await instance.start();
      detector.current = instance;
      await api.setBeatSync(true);
      setActive(true);
    } catch (e) {
      setError(
        e instanceof Error
          ? `Could not start listening: ${e.message}`
          : String(e),
      );
    }
  }

  return (
    <div className="card p-4">
      <div className="mb-2 flex items-center gap-3">
        <h3 className="flex-1 font-semibold">Music sync</h3>
        <button
          className={active ? "btn btn-danger" : "btn btn-ghost"}
          onClick={() => void toggle()}
        >
          {active ? "Stop listening" : "Listen"}
        </button>
      </div>
      <p className="mb-3 text-xs text-[var(--color-ink-400)]">
        Uses your microphone or audio input. Effects marked "per beat" advance one
        step on each detected beat.
      </p>
      <div className="flex items-center gap-3">
        <div
          className="h-2 flex-1 overflow-hidden rounded-full"
          style={{ backgroundColor: "var(--color-ink-700)" }}
        >
          <div
            className="h-full rounded-full"
            style={{
              width: `${level * 100}%`,
              backgroundColor: "var(--color-beam-400)",
              transition: "width 60ms linear",
            }}
          />
        </div>
        <span className="mono w-20 text-right text-sm">
          {bpm ? `${Math.round(bpm)} BPM` : "—"}
        </span>
      </div>
    </div>
  );
}

function EffectCard({
  effect,
  active,
  minDwellMs,
  onRun,
}: {
  effect: Effect;
  active: boolean;
  minDwellMs: number;
  onRun: (effect: Effect) => void;
}) {
  const shortest = Math.min(...effect.steps.map((s) => s.duration_ms));
  // Warn when the car physically cannot render the effect as authored, rather
  // than letting it run and look wrong.
  const tooFast = shortest < Math.max(RENDERABLE_STEP_MS, minDwellMs);

  return (
    <div
      className="card flex flex-col gap-2 p-4"
      style={active ? { borderColor: "var(--color-beam-500)" } : undefined}
    >
      <div className="flex items-start gap-2">
        <div className="flex-1">
          <div className="font-semibold">{effect.name}</div>
          <div className="text-xs text-[var(--color-ink-400)]">
            {effect.steps.length} steps · {shortest} ms shortest
            {effect.timing === "perBeat" && " · per beat"}
          </div>
        </div>
        <button
          className={active ? "btn btn-danger" : "btn btn-primary"}
          onClick={() => onRun(effect)}
        >
          {active ? "Stop" : "Run"}
        </button>
      </div>

      <p className="text-sm text-[var(--color-ink-300)]">{effect.description}</p>

      {tooFast && (
        <p className="text-xs" style={{ color: "var(--color-amber-glow)" }}>
          Steps shorter than {Math.max(RENDERABLE_STEP_MS, minDwellMs)} ms will be
          slowed down: the bus to the headlight modules cannot switch faster.
        </p>
      )}
    </div>
  );
}

export function Effects() {
  const effects = useStore((s) => s.effects);
  const loadEffects = useStore((s) => s.loadEffects);
  const lamps = useStore((s) => s.lamps);
  const loadLamps = useStore((s) => s.loadLamps);
  const lampVisuals = useStore((s) => s.lampVisuals);
  const engine = useStore((s) => s.engine);
  const engineReady = useStore((s) => s.status?.engineReady ?? false);
  const preflight = useStore((s) => s.preflight);
  const setError = useStore((s) => s.setError);
  const refreshEngine = useStore((s) => s.refreshEngine);

  useEffect(() => {
    void loadEffects();
    void loadLamps();
  }, [loadEffects, loadLamps]);

  async function run(effect: Effect) {
    try {
      if (engine?.effectId === effect.id && engine.running) {
        await api.stopEffect();
      } else {
        await api.startEffect(effect);
      }
      await refreshEngine();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  if (!engineReady) {
    return (
      <p className="text-sm text-[var(--color-ink-400)]">
        Connect to a vehicle and load a catalog to run effects.
      </p>
    );
  }

  return (
    <div className="mx-auto grid max-w-6xl gap-4 lg:grid-cols-[1fr_22rem]">
      <div className="space-y-4">
        {!preflight && (
          <div
            className="card p-3 text-sm"
            style={{ borderColor: "var(--color-amber-glow)" }}
          >
            Run the preflight check on the Safety screen before starting an effect.
          </div>
        )}

        <div className="grid gap-3 md:grid-cols-2">
          {effects.map((effect) => (
            <EffectCard
              key={effect.id}
              effect={effect}
              active={engine?.running === true && engine.effectId === effect.id}
              minDwellMs={engine?.minDwellMs ?? 40}
              onRun={(chosen) => void run(chosen)}
            />
          ))}
        </div>
      </div>

      <div className="space-y-4">
        <BeatSync />

        <div className="card p-4">
          <h3 className="mb-2 font-semibold">Live output</h3>
          {engine?.running ? (
            <p className="mb-3 text-xs text-[var(--color-ink-400)]">
              Step {engine.stepIndex + 1} of {engine.stepCount}
            </p>
          ) : (
            <p className="mb-3 text-xs text-[var(--color-ink-400)]">Idle.</p>
          )}
          <LampGrid lamps={lamps} levels={lampVisuals} featuredOnly />
        </div>
      </div>
    </div>
  );
}
