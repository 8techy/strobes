/**
 * Vehicle screen: read-only diagnostics.
 *
 * Everything here is safe to run on any car at any time — module probes and DTC
 * reads change nothing. This is deliberately the first thing that works, so a
 * connection can be proven before anything is actuated.
 */

import { useEffect, useState } from "react";

import { useStore } from "../store";
import * as api from "../api";

function EcuTable() {
  const ecus = useStore((s) => s.ecus);
  const scanEcus = useStore((s) => s.scanEcus);
  const connected = useStore((s) => s.status?.connected ?? false);
  const [scanning, setScanning] = useState(false);

  async function scan() {
    setScanning(true);
    await scanEcus();
    setScanning(false);
  }

  return (
    <div className="card p-4">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="flex-1 text-base font-bold">Modules</h2>
        <button
          className="btn btn-ghost"
          disabled={!connected || scanning}
          onClick={() => void scan()}
        >
          {scanning ? "Probing" : "Scan"}
        </button>
      </div>

      {ecus.length === 0 ? (
        <p className="text-sm text-[var(--color-ink-400)]">
          Scan to see which lighting modules this car has. Probes are read-only.
        </p>
      ) : (
        <div className="space-y-1">
          {ecus.map((ecu) => (
            <div
              key={ecu.address}
              className="flex items-center gap-3 rounded-lg px-3 py-1.5 text-sm"
              style={{ backgroundColor: "var(--color-ink-850)" }}
            >
              <span
                aria-hidden
                className="size-2 shrink-0 rounded-full"
                style={{
                  backgroundColor: ecu.present
                    ? "var(--color-safe)"
                    : "var(--color-ink-600)",
                }}
              />
              <span className="mono w-14 shrink-0 text-[var(--color-ink-300)]">
                {ecu.address_hex}
              </span>
              <span className="flex-1">{ecu.label}</span>
              <span className="mono text-xs text-[var(--color-ink-400)]">
                {ecu.serial ?? ecu.note ?? ""}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function DtcPanel() {
  const dtcs = useStore((s) => s.dtcs);
  const loadDtcs = useStore((s) => s.loadDtcs);
  const setError = useStore((s) => s.setError);
  const connected = useStore((s) => s.status?.connected ?? false);
  const [busy, setBusy] = useState(false);

  async function clear() {
    setBusy(true);
    try {
      await api.clearDtcs();
      await loadDtcs();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card p-4">
      <div className="mb-3 flex items-center gap-3">
        <h2 className="flex-1 text-base font-bold">Fault codes</h2>
        <button
          className="btn btn-ghost"
          disabled={!connected}
          onClick={() => void loadDtcs()}
        >
          Read
        </button>
        <button
          className="btn btn-ghost"
          disabled={!connected || busy || dtcs.length === 0}
          onClick={() => void clear()}
          title="Clears stored codes on the body controller"
        >
          Clear
        </button>
      </div>

      {dtcs.length === 0 ? (
        <p className="text-sm text-[var(--color-ink-400)]">
          No codes read yet. Reading is safe and changes nothing.
        </p>
      ) : (
        <div className="space-y-1">
          {dtcs.map((dtc) => (
            <div
              key={dtc.code}
              className="flex items-center gap-3 rounded-lg px-3 py-1.5 text-sm"
              style={{ backgroundColor: "var(--color-ink-850)" }}
            >
              <span className="mono w-24 shrink-0 font-semibold">{dtc.code_hex}</span>
              <span className="flex-1 text-xs text-[var(--color-ink-300)]">
                {dtc.confirmed && "present now"}
                {dtc.confirmed && dtc.pending && " · "}
                {dtc.pending && "seen this ignition cycle"}
              </span>
              {dtc.warning_indicator && (
                <span
                  className="pill"
                  style={{
                    borderColor: "var(--color-amber-glow)",
                    color: "var(--color-amber-glow)",
                  }}
                >
                  warning shown
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Vehicle() {
  const vehicle = useStore((s) => s.vehicle);
  const connected = useStore((s) => s.status?.connected ?? false);
  const scanEcus = useStore((s) => s.scanEcus);
  const loadDtcs = useStore((s) => s.loadDtcs);

  // Populate on arrival so the screen is useful immediately.
  useEffect(() => {
    if (!connected) return;
    void scanEcus();
    void loadDtcs();
  }, [connected, scanEcus, loadDtcs]);

  if (!connected) {
    return (
      <p className="text-sm text-[var(--color-ink-400)]">
        Not connected. Use the Connect screen first.
      </p>
    );
  }

  return (
    <div className="mx-auto max-w-5xl space-y-4">
      {vehicle && (
        <div className="card flex flex-wrap items-center gap-x-8 gap-y-2 p-4 text-sm">
          <div>
            <div className="label">VIN</div>
            <div className="mono">{vehicle.vin ?? "not reported"}</div>
          </div>
          <div>
            <div className="label">Transport</div>
            <div>{vehicle.protocol}</div>
          </div>
          <div>
            <div className="label">Gateway serial</div>
            <div className="mono">{vehicle.gateway_serial ?? "not reported"}</div>
          </div>
        </div>
      )}
      <EcuTable />
      <DtcPanel />
    </div>
  );
}
