/**
 * Safety screen: the preflight check and what it found.
 *
 * This exists as its own screen rather than a modal because the short-circuit
 * counter reading is the single most consequential thing the app does. An output
 * whose counter reaches its limit is permanently disabled and the module has to
 * be replaced, so the user should be able to look at this deliberately.
 */

import { useState } from "react";

import * as api from "../api";
import { useStore } from "../store";

/** BMW's per-lamp short-circuit shutdown limit. */
const SHORT_CIRCUIT_LIMIT = 50;

function CounterBar({ counter }: { counter: number }) {
  const fraction = Math.min(1, counter / SHORT_CIRCUIT_LIMIT);
  const colour =
    counter >= SHORT_CIRCUIT_LIMIT
      ? "var(--color-danger)"
      : counter > SHORT_CIRCUIT_LIMIT / 2
        ? "var(--color-amber-glow)"
        : "var(--color-ink-400)";

  return (
    <div className="flex items-center gap-2">
      <div
        className="h-1.5 w-24 overflow-hidden"
        style={{ backgroundColor: "var(--color-ink-700)" }}
      >
        <div
          className="h-full"
          style={{ width: `${fraction * 100}%`, backgroundColor: colour }}
        />
      </div>
      <span className="mono text-xs" style={{ color: colour }}>
        {counter}/{SHORT_CIRCUIT_LIMIT}
      </span>
    </div>
  );
}

export function Safety() {
  const preflight = useStore((s) => s.preflight);
  const runPreflight = useStore((s) => s.runPreflight);
  const loadDtcs = useStore((s) => s.loadDtcs);
  const setError = useStore((s) => s.setError);
  const connected = useStore((s) => s.status?.connected ?? false);
  const [busy, setBusy] = useState<"preflight" | "clear" | null>(null);

  async function run() {
    setBusy("preflight");
    await runPreflight();
    setBusy(null);
  }

  async function clearCodes() {
    setBusy("clear");
    try {
      await api.clearDtcs();
      await loadDtcs();
      await runPreflight();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  const hasStoredCodes = (preflight?.dtcs_before.length ?? 0) > 0;

  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div className="card p-4">
        <div className="mb-2 flex flex-wrap items-center gap-3">
          <h2 className="flex-1 text-base font-bold">Preflight check</h2>
          <button
            className="btn btn-ghost"
            disabled={!connected || busy !== null}
            onClick={() => void clearCodes()}
            title="Clears stored fault codes on the body controller"
          >
            {busy === "clear" ? "Clearing" : "Clear codes"}
          </button>
          <button
            className="btn btn-primary"
            disabled={!connected || busy !== null}
            onClick={() => void run()}
          >
            {busy === "preflight" ? "Checking" : "Run preflight"}
          </button>
        </div>
        <p className="text-sm text-[var(--color-ink-300)]">
          Reads stored fault codes and the per-lamp short-circuit counters. Effects
          will not start until this has passed, and any output the car has already
          recorded faults against is excluded.
        </p>
      </div>

      {preflight && (
        <>
          <div
            className="card flex items-center gap-3 p-4"
            style={{
              borderColor: preflight.passed
                ? "var(--color-safe)"
                : "var(--color-danger)",
            }}
          >
            <span
              aria-hidden
              className="size-3"
              style={{
                backgroundColor: preflight.passed
                  ? "var(--color-safe)"
                  : "var(--color-danger)",
              }}
            />
            <span className="font-semibold">
              {preflight.passed
                ? "Safe to run effects"
                : "Effects are blocked until these are resolved"}
            </span>
          </div>

          {preflight.blockers.length > 0 && (
            <div className="card p-4">
              <h3 className="mb-2 font-semibold text-[var(--color-danger)]">
                Blocking problems
              </h3>
              <ul className="space-y-1.5 text-sm">
                {preflight.blockers.map((blocker) => (
                  <li key={blocker}>{blocker}</li>
                ))}
              </ul>
            </div>
          )}

          {preflight.warnings.length > 0 && (
            <div className="card p-4">
              <h3 className="mb-2 font-semibold text-[var(--color-amber-glow)]">
                Worth knowing
              </h3>
              <ul className="space-y-1.5 text-sm text-[var(--color-ink-300)]">
                {preflight.warnings.map((warning) => (
                  <li key={warning}>{warning}</li>
                ))}
              </ul>
            </div>
          )}

          <div className="card p-4">
            <h3 className="mb-1 font-semibold">Short-circuit counters</h3>
            <p className="mb-3 text-xs text-[var(--color-ink-400)]">
              The body controller increments an output's counter once per ignition
              cycle in which it detects a fault. At {SHORT_CIRCUIT_LIMIT} the output
              is disabled permanently.
            </p>
            {preflight.degraded_lamps.length === 0 ? (
              <p className="text-sm text-[var(--color-safe)]">
                Every output the car reported reads zero.
              </p>
            ) : (
              <div className="space-y-1">
                {preflight.degraded_lamps.map((lamp) => (
                  <div
                    key={lamp.lamp}
                    className="flex items-center gap-3 px-3 py-1.5 text-sm"
                    style={{ backgroundColor: "var(--color-ink-850)" }}
                  >
                    <span className="flex-1">{lamp.name}</span>
                    <span className="mono text-xs text-[var(--color-ink-400)]">
                      {lamp.code}
                    </span>
                    <CounterBar counter={lamp.counter} />
                    {lamp.locked_out && (
                      <span
                        className="pill"
                        style={{
                          borderColor: "var(--color-danger)",
                          color: "var(--color-danger)",
                        }}
                      >
                        locked out
                      </span>
                    )}
                  </div>
                ))}
              </div>
            )}
          </div>

          <div className="card p-4">
            <div className="mb-1 flex flex-wrap items-center gap-3">
              <h3 className="flex-1 font-semibold">Faults present before connecting</h3>
              {hasStoredCodes && (
                <button
                  className="btn btn-ghost"
                  disabled={!connected || busy !== null}
                  onClick={() => void clearCodes()}
                  title="Clears stored fault codes on the body controller"
                >
                  {busy === "clear" ? "Clearing" : "Clear codes"}
                </button>
              )}
            </div>
            <p className="mb-3 text-xs text-[var(--color-ink-400)]">
              Recorded so you can tell pre-existing codes apart from anything new.
            </p>
            {preflight.dtcs_before.length === 0 ? (
              <p className="text-sm text-[var(--color-safe)]">None stored.</p>
            ) : (
              <div className="mono flex flex-wrap gap-2 text-xs">
                {preflight.dtcs_before.map((dtc) => (
                  <span key={dtc.code} className="pill">
                    {dtc.code_hex}
                  </span>
                ))}
              </div>
            )}
          </div>
        </>
      )}

      <div className="card p-4 text-sm leading-relaxed text-[var(--color-ink-300)]">
        <h3 className="mb-2 font-semibold text-[var(--color-ink-100)]">
          What Strobes will not do
        </h3>
        <ul className="space-y-1.5">
          <li>
            No persistent changes. Everything runs through short-term actuation, so
            losing the connection reverts the car automatically.
          </li>
          <li>
            No coding, no flashing, and no programming session. Those requests are
            refused before they reach the wire.
          </li>
          <li>Nothing is ever sent to the engine control module.</li>
          <li>
            Some cars refuse light control while moving or with the engine
            running. Strobes still tries; if the body module rejects a request,
            the reason is shown in the banner.
          </li>
        </ul>
      </div>
    </div>
  );
}
