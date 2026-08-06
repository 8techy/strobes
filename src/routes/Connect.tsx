/**
 * Connect screen: find a vehicle, pick a catalog, or start the simulator.
 *
 * Also the place where the physical setup is explained, because a missing pin 8
 * pull-up is the single most common reason an ENET cable never responds and it
 * is invisible from software.
 */

import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";

import * as api from "../api";
import { useStore } from "../store";
import type { DiscoveredVehicle, Protocol } from "../types";

function SetupHelp() {
  return (
    <div className="card p-4 text-sm leading-relaxed text-[var(--color-ink-300)]">
      <h3 className="mb-2 font-semibold text-[var(--color-ink-100)]">
        Before you connect
      </h3>
      <ul className="space-y-1.5">
        <li>
          Your car has a 16-pin OBD-II port, not USB-C. An ENET cable is
          RJ45-to-OBD, so on a modern laptop it goes through a USB-C Ethernet
          adapter.
        </li>
        <li>
          The cable must bridge OBD pin 8 to pin 16 through a 510 ohm resistor.
          Without it the car's Ethernet stays powered down and nothing responds.
          Commercial ENET cables already include it.
        </li>
        <li>
          Ignition on, engine off. Connect a battery charger: low voltage during
          actuation is the classic cause of module faults.
        </li>
        <li>
          F-series speaks HSFZ, G-series speaks DoIP. Discovery probes both, so
          you do not need to know which.
        </li>
      </ul>
    </div>
  );
}

function SimulatorCard() {
  const startSimulator = useStore((s) => s.startSimulator);
  const stopSimulator = useStore((s) => s.stopSimulator);
  const setError = useStore((s) => s.setError);
  const connect = useStore((s) => s.connect);
  const simulatorRunning = useStore((s) => s.status?.simulatorRunning ?? false);
  const address = useStore((s) => s.simulatorAddress);
  const [busy, setBusy] = useState(false);

  async function launch(protocol: Protocol) {
    setBusy(true);
    try {
      const target = await startSimulator(protocol);
      const [host, port] = target.split(":");
      if (host && port) {
        await connect(protocol, host, Number(port));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card p-4">
      <h3 className="mb-1 font-semibold">No car handy?</h3>
      <p className="mb-3 text-sm text-[var(--color-ink-300)]">
        The built-in simulator answers real HSFZ and DoIP traffic, so you can
        explore everything without a vehicle. Load the{" "}
        <code className="mono text-xs">SIM</code> catalog once it is running.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <button
          className="btn btn-ghost"
          disabled={busy || simulatorRunning}
          onClick={() => void launch("hsfz")}
        >
          Start as F-series (HSFZ)
        </button>
        <button
          className="btn btn-ghost"
          disabled={busy || simulatorRunning}
          onClick={() => void launch("doip")}
        >
          Start as G-series (DoIP)
        </button>
        {simulatorRunning && (
          <>
            <span className="pill mono">{address ?? "running"}</span>
            <button className="btn btn-ghost" onClick={() => void stopSimulator()}>
              Stop
            </button>
          </>
        )}
      </div>
    </div>
  );
}

function CatalogPicker() {
  const catalogs = useStore((s) => s.catalogs);
  const loadCatalogs = useStore((s) => s.loadCatalogs);
  const chooseCatalog = useStore((s) => s.chooseCatalog);
  const activeId = useStore((s) => s.status?.catalogId ?? null);

  useEffect(() => {
    void loadCatalogs();
  }, [loadCatalogs]);

  if (catalogs.length === 0) {
    return (
      <div className="card p-4 text-sm text-[var(--color-ink-300)]">
        No catalogs found. They are expected in the <code className="mono">catalog/</code>{" "}
        directory next to the application.
      </div>
    );
  }

  return (
    <div className="card p-4">
      <h3 className="mb-1 font-semibold">Catalog</h3>
      <p className="mb-3 text-sm text-[var(--color-ink-300)]">
        A catalog maps lamps to the diagnostic identifiers your chassis uses.
      </p>
      <div className="space-y-2">
        {catalogs.map((catalog) => {
          const active = catalog.chassisId === activeId;
          return (
            <button
              key={catalog.path}
              onClick={() => void chooseCatalog(catalog.path)}
              className="flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors"
              style={{
                backgroundColor: active
                  ? "var(--color-ink-700)"
                  : "var(--color-ink-850)",
                border: `1px solid ${active ? "var(--color-beam-500)" : "var(--color-ink-700)"}`,
              }}
            >
              <div className="flex-1">
                <div className="font-semibold">{catalog.name}</div>
                <div className="text-xs text-[var(--color-ink-400)]">
                  {catalog.chassisId} · {catalog.transport} · {catalog.actionCount}{" "}
                  action{catalog.actionCount === 1 ? "" : "s"}
                </div>
              </div>
              <span
                className="pill"
                style={
                  catalog.verified
                    ? { borderColor: "var(--color-safe)", color: "var(--color-safe)" }
                    : {
                        borderColor: "var(--color-amber-glow)",
                        color: "var(--color-amber-glow)",
                      }
                }
              >
                {catalog.verified ? "verified" : "placeholders"}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

export function Connect() {
  const connect = useStore((s) => s.connect);
  const disconnect = useStore((s) => s.disconnect);
  const connecting = useStore((s) => s.connecting);
  const status = useStore((s) => s.status);
  const vehicle = useStore((s) => s.vehicle);
  const setError = useStore((s) => s.setError);
  const navigate = useNavigate();

  const [found, setFound] = useState<DiscoveredVehicle[]>([]);
  const [scanning, setScanning] = useState(false);
  const [host, setHost] = useState("");
  const [protocol, setProtocol] = useState<Protocol>("hsfz");

  async function discover() {
    setScanning(true);
    setFound([]);
    try {
      setFound(await api.discoverVehicles());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setScanning(false);
    }
  }

  return (
    <div className="mx-auto grid max-w-6xl gap-4 lg:grid-cols-2">
      <div className="space-y-4">
        <div className="card p-4">
          <h2 className="mb-3 text-base font-bold">Find a vehicle</h2>

          <div className="mb-3 flex items-center gap-2">
            <button
              className="btn btn-primary"
              onClick={() => void discover()}
              disabled={scanning}
            >
              {scanning ? "Listening" : "Scan the network"}
            </button>
            {status?.connected && (
              <button className="btn btn-ghost" onClick={() => void disconnect()}>
                Disconnect
              </button>
            )}
          </div>

          {found.length > 0 && (
            <div className="mb-4 space-y-2">
              {found.map((item) => (
                <div
                  key={`${item.ip}:${item.port}`}
                  className="flex items-center gap-3 rounded-lg px-3 py-2"
                  style={{
                    backgroundColor: "var(--color-ink-850)",
                    border: "1px solid var(--color-ink-700)",
                  }}
                >
                  <div className="flex-1 text-sm">
                    <div className="mono font-semibold">{item.ip}</div>
                    <div className="text-xs text-[var(--color-ink-400)]">
                      {item.protocol.toUpperCase()} · port {item.port}
                      {item.vin && ` · VIN ${item.vin}`}
                    </div>
                  </div>
                  <button
                    className="btn btn-primary"
                    disabled={connecting}
                    onClick={() => void connect(item.protocol, item.ip, item.port)}
                  >
                    Connect
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="space-y-2">
            <span className="label">Or enter an address</span>
            <div className="flex gap-2">
              <select
                className="input max-w-32"
                value={protocol}
                onChange={(e) => setProtocol(e.target.value as Protocol)}
              >
                <option value="hsfz">HSFZ</option>
                <option value="doip">DoIP</option>
              </select>
              <input
                className="input"
                placeholder="169.254.87.130"
                value={host}
                onChange={(e) => setHost(e.target.value)}
              />
              <button
                className="btn btn-ghost"
                disabled={!host || connecting}
                onClick={() => void connect(protocol, host)}
              >
                Connect
              </button>
            </div>
          </div>
        </div>

        {vehicle && (
          <div className="card p-4">
            <h3 className="mb-2 font-semibold">Connected</h3>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
              <dt className="text-[var(--color-ink-400)]">VIN</dt>
              <dd className="mono">{vehicle.vin ?? "not reported"}</dd>
              <dt className="text-[var(--color-ink-400)]">Protocol</dt>
              <dd>{vehicle.protocol}</dd>
              <dt className="text-[var(--color-ink-400)]">Gateway serial</dt>
              <dd className="mono">{vehicle.gateway_serial ?? "not reported"}</dd>
            </dl>
            <button
              className="btn btn-primary mt-3"
              onClick={() => void navigate("/vehicle")}
            >
              Inspect modules
            </button>
          </div>
        )}

        <CatalogPicker />
      </div>

      <div className="space-y-4">
        <SetupHelp />
        <SimulatorCard />
      </div>
    </div>
  );
}
