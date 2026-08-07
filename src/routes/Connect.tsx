/**
 * Connect screen: find a vehicle and open a session.
 *
 * Catalog choice is automatic from the protocol (HSFZ → F-series, DoIP →
 * G-series). A short confirm runs before each connect so the cable or adapter
 * is checked first.
 */

import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";

import * as api from "../api";
import { useStore } from "../store";
import type { DiscoveredVehicle, Protocol } from "../types";

type PendingConnect = {
  protocol: Protocol;
  host: string;
  port?: number;
};

/** Match the chassis catalog to the protocol the user is about to use. */
function catalogForProtocol(
  catalogs: { path: string; chassisId: string; transport: string }[],
  protocol: Protocol,
) {
  const real = catalogs.filter((c) => c.chassisId !== "SIM");
  return (
    real.find((c) => c.transport.toLowerCase() === protocol) ?? real[0] ?? null
  );
}

function ConnectConfirm({
  pending,
  onCancel,
  onConfirm,
}: {
  pending: PendingConnect;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4"
      style={{ backgroundColor: "rgba(0, 0, 0, 0.72)" }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="connect-confirm-title"
    >
      <div className="card w-full max-w-md p-5">
        <h3
          id="connect-confirm-title"
          className="mb-2 text-base font-bold text-[var(--color-ink-100)]"
        >
          Ready to connect?
        </h3>
        <p className="mb-1 text-sm leading-relaxed text-[var(--color-ink-300)]">
          Make sure your ENET cable or Bluetooth adapter is plugged in and
          linked to the car before you continue.
        </p>
        <p className="mb-4 text-sm leading-relaxed text-[var(--color-ink-400)]">
          Ignition on, engine off. Target{" "}
          <span className="mono text-[var(--color-ink-300)]">{pending.host}</span>
          {pending.port != null ? `:${pending.port}` : ""} over{" "}
          {pending.protocol.toUpperCase()}.
        </p>
        <div className="flex justify-end gap-2">
          <button className="btn btn-ghost" onClick={onCancel}>
            Cancel
          </button>
          <button className="btn btn-primary" onClick={onConfirm}>
            Connect
          </button>
        </div>
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
  const catalogs = useStore((s) => s.catalogs);
  const loadCatalogs = useStore((s) => s.loadCatalogs);
  const chooseCatalog = useStore((s) => s.chooseCatalog);
  const setError = useStore((s) => s.setError);
  const navigate = useNavigate();

  const [found, setFound] = useState<DiscoveredVehicle[]>([]);
  const [scanning, setScanning] = useState(false);
  const [host, setHost] = useState("");
  const [protocol, setProtocol] = useState<Protocol>("hsfz");
  const [pending, setPending] = useState<PendingConnect | null>(null);

  useEffect(() => {
    void loadCatalogs();
  }, [loadCatalogs]);

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

  function requestConnect(next: PendingConnect) {
    setPending(next);
  }

  async function confirmConnect() {
    if (!pending) return;
    const target = pending;
    setPending(null);

    let available = catalogs;
    if (available.length === 0) {
      await loadCatalogs();
      available = useStore.getState().catalogs;
    }

    const catalog = catalogForProtocol(available, target.protocol);
    if (catalog && status?.catalogId !== catalog.chassisId) {
      await chooseCatalog(catalog.path);
    }

    await connect(target.protocol, target.host, target.port);
  }

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col justify-center gap-4 py-6 lg:min-h-[calc(100vh-8rem)]">
      <div className="card p-5">
        <h2 className="mb-3 text-base font-bold">Find a vehicle</h2>

        <div className="mb-3 flex flex-wrap items-center gap-2">
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
                className="flex items-center gap-3 px-3 py-2"
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
                  onClick={() =>
                    requestConnect({
                      protocol: item.protocol,
                      host: item.ip,
                      port: item.port,
                    })
                  }
                >
                  Connect
                </button>
              </div>
            ))}
          </div>
        )}

        <div className="space-y-2">
          <span className="label">Or enter an address</span>
          <div className="flex flex-wrap gap-2">
            <select
              className="input max-w-34"
              value={protocol}
              onChange={(e) => setProtocol(e.target.value as Protocol)}
            >
              <option value="hsfz">HSFZ (F-Series)</option>
              <option value="doip">DoIP (G-Series)</option>
            </select>
            <input
              className="input min-w-0 flex-1"
              placeholder="169.254.87.130"
              value={host}
              onChange={(e) => setHost(e.target.value)}
            />
            <button
              className="btn btn-ghost"
              disabled={!host || connecting}
              onClick={() => requestConnect({ protocol, host })}
            >
              Connect
            </button>
          </div>
        </div>
      </div>

      {vehicle && (
        <div className="card p-5">
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

      {pending && (
        <ConnectConfirm
          pending={pending}
          onCancel={() => setPending(null)}
          onConfirm={() => void confirmConnect()}
        />
      )}
    </div>
  );
}
