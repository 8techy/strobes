/**
 * Editor screen: build an effect step by step.
 *
 * Surfaces the car's timing floor directly in the duration control, so it is not
 * possible to author something the vehicle silently fails to render.
 */

import { useEffect, useState } from "react";

import * as api from "../api";
import { LampGrid } from "../components/LampGrid";
import { useStore } from "../store";
import type { Effect, LampInfo, Step } from "../types";

/** Below this the LIN bus to the headlight modules cannot keep up. */
const RENDERABLE_STEP_MS = 20;

function emptyStep(durationMs: number): Step {
  return { commands: [], duration_ms: durationMs };
}

export function Editor() {
  const lamps = useStore((s) => s.lamps);
  const loadLamps = useStore((s) => s.loadLamps);
  const engine = useStore((s) => s.engine);
  const engineReady = useStore((s) => s.status?.engineReady ?? false);
  const preflight = useStore((s) => s.preflight);
  const setError = useStore((s) => s.setError);
  const lampVisuals = useStore((s) => s.lampVisuals);

  const floor = Math.max(RENDERABLE_STEP_MS, engine?.minDwellMs ?? 40);

  const [name, setName] = useState("My effect");
  const [looping, setLooping] = useState(true);
  const [perBeat, setPerBeat] = useState(false);
  const [steps, setSteps] = useState<Step[]>([emptyStep(200), emptyStep(200)]);
  const [activeStep, setActiveStep] = useState(0);
  const [showRegulated, setShowRegulated] = useState(false);

  useEffect(() => {
    void loadLamps();
  }, [loadLamps]);

  const degraded = new Set(preflight?.degraded_lamps.map((l) => l.lamp) ?? []);
  const current = steps[activeStep];
  const selected = new Set(
    current?.commands.filter((c) => c.level > 0).map((c) => c.lamp) ?? [],
  );

  /** Toggles a lamp within the active step. */
  function toggleLamp(lamp: LampInfo) {
    setSteps((previous) =>
      previous.map((step, index) => {
        if (index !== activeStep) return step;
        const existing = step.commands.find((c) => c.lamp === lamp.id);
        if (existing) {
          return {
            ...step,
            commands: step.commands.filter((c) => c.lamp !== lamp.id),
          };
        }
        return {
          ...step,
          commands: [...step.commands, { lamp: lamp.id, level: 100 }],
        };
      }),
    );
  }

  function setDuration(index: number, value: number) {
    setSteps((previous) =>
      previous.map((step, i) =>
        i === index ? { ...step, duration_ms: Math.max(floor, value) } : step,
      ),
    );
  }

  function addStep() {
    setSteps((previous) => [...previous, emptyStep(Math.max(floor, 200))]);
    setActiveStep(steps.length);
  }

  function removeStep(index: number) {
    if (steps.length <= 1) return;
    setSteps((previous) => previous.filter((_, i) => i !== index));
    setActiveStep((prev) => Math.max(0, Math.min(prev, steps.length - 2)));
  }

  function build(): Effect {
    return {
      id: `custom-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`,
      name,
      description: "Built in the editor.",
      steps,
      looping,
      timing: perBeat ? "perBeat" : "fixed",
    };
  }

  async function run() {
    try {
      if (engine?.running) {
        await api.stopEffect();
        return;
      }
      await api.startEffect(build());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  function exportJson() {
    const json = JSON.stringify(build(), null, 2);
    void navigator.clipboard.writeText(json).catch(() => {
      setError("Could not copy to the clipboard.");
    });
  }

  const empty = steps.every((step) => step.commands.length === 0);

  return (
    <div className="mx-auto grid max-w-6xl gap-4 lg:grid-cols-[24rem_1fr]">
      <div className="space-y-4">
        <div className="card space-y-3 p-4">
          <div>
            <label className="label" htmlFor="effect-name">
              Name
            </label>
            <input
              id="effect-name"
              className="input"
              value={name}
              onChange={(e) => setName(e.target.value)}
            />
          </div>

          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={looping}
              onChange={(e) => setLooping(e.target.checked)}
            />
            Loop
          </label>

          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={perBeat}
              onChange={(e) => setPerBeat(e.target.checked)}
            />
            Advance one step per beat
          </label>

          <div className="flex gap-2 pt-1">
            <button
              className={engine?.running ? "btn btn-danger" : "btn btn-primary"}
              disabled={!engineReady || empty}
              onClick={() => void run()}
            >
              {engine?.running ? "Stop" : "Run"}
            </button>
            <button className="btn btn-ghost" onClick={exportJson}>
              Copy JSON
            </button>
          </div>

          {empty && (
            <p className="text-xs text-[var(--color-ink-400)]">
              Add at least one lamp to a step before running.
            </p>
          )}
        </div>

        <div className="card p-4">
          <div className="mb-3 flex items-center gap-2">
            <h3 className="flex-1 font-semibold">Steps</h3>
            <button className="btn btn-ghost" onClick={addStep}>
              Add
            </button>
          </div>

          <div className="space-y-2">
            {steps.map((step, index) => (
              <div
                key={index}
                className="rounded-lg p-2"
                style={{
                  backgroundColor:
                    index === activeStep
                      ? "var(--color-ink-700)"
                      : "var(--color-ink-850)",
                  border: `1px solid ${
                    index === activeStep
                      ? "var(--color-beam-500)"
                      : "var(--color-ink-700)"
                  }`,
                }}
              >
                <div className="flex items-center gap-2">
                  <button
                    className="flex-1 text-left text-sm font-semibold"
                    onClick={() => setActiveStep(index)}
                  >
                    Step {index + 1}
                    <span className="ml-2 text-xs font-normal text-[var(--color-ink-400)]">
                      {step.commands.length} lamp
                      {step.commands.length === 1 ? "" : "s"}
                    </span>
                  </button>
                  <button
                    className="text-xs text-[var(--color-ink-400)] hover:text-[var(--color-danger)]"
                    disabled={steps.length <= 1}
                    onClick={() => removeStep(index)}
                  >
                    Remove
                  </button>
                </div>

                <div className="mt-2 flex items-center gap-2">
                  <input
                    type="range"
                    min={floor}
                    max={1000}
                    step={10}
                    value={step.duration_ms}
                    onChange={(e) => setDuration(index, Number(e.target.value))}
                    className="flex-1"
                  />
                  <span className="mono w-16 text-right text-xs">
                    {step.duration_ms} ms
                  </span>
                </div>
              </div>
            ))}
          </div>

          <p className="mt-3 text-xs text-[var(--color-ink-400)]">
            Minimum {floor} ms per step on this car. Anything faster cannot be
            rendered by the lighting modules.
          </p>
        </div>
      </div>

      <div className="card p-4">
        <div className="mb-3 flex items-center gap-3">
          <h3 className="flex-1 font-semibold">
            Lamps in step {activeStep + 1}
          </h3>
          <label className="flex items-center gap-2 text-xs">
            <input
              type="checkbox"
              checked={showRegulated}
              onChange={(e) => setShowRegulated(e.target.checked)}
            />
            Show regulated devices
          </label>
        </div>

        <LampGrid
          lamps={lamps}
          levels={engine?.running ? lampVisuals : Object.fromEntries(
            [...selected].map((id) => [id, 100]),
          )}
          selected={selected}
          degraded={degraded}
          onPick={toggleLamp}
          featuredOnly={!showRegulated}
        />
      </div>
    </div>
  );
}
