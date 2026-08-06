/**
 * Application chrome: navigation, connection indicator and the panic control.
 *
 * The panic button lives in the chrome rather than on any single screen because
 * it must be reachable from everywhere the moment something looks wrong.
 */

import { NavLink, Outlet } from "react-router-dom";
import { useEffect, useState } from "react";

import * as api from "../api";
import { useStore } from "../store";
import {
  applyTheme,
  getStoredTheme,
  toggleTheme,
  type Theme,
} from "../theme";

const NAV = [
  { to: "/", label: "Connect", hint: "Find and connect to a vehicle" },
  { to: "/vehicle", label: "Vehicle", hint: "VIN, modules and fault codes" },
  { to: "/effects", label: "Effects", hint: "Run a light show" },
  { to: "/editor", label: "Editor", hint: "Build your own effect" },
  { to: "/lab", label: "Lab", hint: "Drive one lamp at a time" },
  { to: "/safety", label: "Safety", hint: "Preflight and short-circuit counters" },
];

function ConnectionPill() {
  const status = useStore((s) => s.status);

  if (!status?.connected) {
    return <span className="pill">Not connected</span>;
  }
  return (
    <span
      className="pill"
      style={{ borderColor: "var(--color-safe)", color: "var(--color-safe)" }}
    >
      {status.protocol} · {status.host}
    </span>
  );
}

/** Warns whenever the loaded catalog has unverified identifiers. */
function CatalogPill() {
  const status = useStore((s) => s.status);
  if (!status?.catalogId) return null;

  const verified = status.catalogVerified;
  return (
    <span
      className="pill"
      title={
        verified
          ? "Every identifier in this catalog has been confirmed on a vehicle."
          : "This catalog contains placeholder identifiers. Effects stay disabled until they are verified."
      }
      style={
        verified
          ? undefined
          : { borderColor: "var(--color-amber-glow)", color: "var(--color-amber-glow)" }
      }
    >
      {status.catalogId}
      {!verified && " · unverified"}
    </span>
  );
}

function PanicButton() {
  const [busy, setBusy] = useState(false);
  const setError = useStore((s) => s.setError);
  const clearLampVisuals = useStore((s) => s.clearLampVisuals);
  const refreshEngine = useStore((s) => s.refreshEngine);
  const engineReady = useStore((s) => s.status?.engineReady ?? false);

  async function stop() {
    setBusy(true);
    try {
      await api.panicStop();
      clearLampVisuals();
      await refreshEngine();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <button
      className="btn btn-panic"
      onClick={stop}
      disabled={busy || !engineReady}
      title="Immediately hand every light back to the car"
    >
      {busy ? "Stopping" : "Panic"}
    </button>
  );
}

function ErrorBanner() {
  const error = useStore((s) => s.error);
  const setError = useStore((s) => s.setError);
  if (!error) return null;

  return (
    <div
      className="flex items-start gap-3 px-4 py-2 text-sm"
      style={{
        backgroundColor: "color-mix(in srgb, var(--color-danger) 14%, transparent)",
        borderBottom: "1px solid var(--color-danger)",
      }}
    >
      <span className="flex-1">{error}</span>
      <button className="text-xs underline" onClick={() => setError(null)}>
        Dismiss
      </button>
    </div>
  );
}

function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(() => getStoredTheme());

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  return (
    <button
      type="button"
      className="theme-toggle"
      onClick={() => setTheme((current) => toggleTheme(current))}
      title={
        theme === "dark"
          ? "Switch to soft light theme"
          : "Switch to OLED dark theme"
      }
      aria-label={
        theme === "dark" ? "Switch to light theme" : "Switch to dark theme"
      }
    >
      <span aria-hidden>{theme === "dark" ? "○" : "●"}</span>
      {theme === "dark" ? "Light" : "Dark"}
    </button>
  );
}

export function Shell() {
  const refreshStatus = useStore((s) => s.refreshStatus);
  const refreshEngine = useStore((s) => s.refreshEngine);
  const applyLampVisual = useStore((s) => s.applyLampVisual);
  const clearLampVisuals = useStore((s) => s.clearLampVisuals);
  const setError = useStore((s) => s.setError);

  // Poll status rather than pushing it: it changes rarely and a poll keeps the
  // UI correct even if an event is missed during a reconnect.
  useEffect(() => {
    void refreshStatus();
    void refreshEngine();
    const timer = window.setInterval(() => {
      void refreshStatus();
      void refreshEngine();
    }, 2000);
    return () => window.clearInterval(timer);
  }, [refreshStatus, refreshEngine]);

  // Mirror engine steps onto the on-screen car so the preview matches reality.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void api
      .onEngineEvent((event) => {
        if (event.type === "step") {
          for (const command of event.commands) {
            applyLampVisual(command.lamp, command.level);
          }
        } else if (event.type === "released" || event.type === "stopped") {
          clearLampVisuals();
        }
      })
      .then((fn) => {
        if (cancelled) {
          fn();
          return;
        }
        unlisten = fn;
      })
      .catch((e) => {
        if (cancelled) return;
        setError(
          e instanceof Error
            ? `Could not subscribe to engine events: ${e.message}`
            : String(e),
        );
      });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [applyLampVisual, clearLampVisuals, setError]);

  return (
    <div className="flex h-full flex-col">
      <header
        className="flex items-center gap-5 px-4 py-3"
        style={{ borderBottom: "1px solid var(--color-ink-700)" }}
      >
        <NavLink to="/" className="shrink-0" title="Strobes">
          <img
            src="/strobes-logo.png"
            alt="Strobes"
            className="brand-logo"
          />
        </NavLink>

        <nav className="flex flex-1 items-center gap-0.5 overflow-x-auto">
          {NAV.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              title={item.hint}
              end={item.to === "/"}
              className={({ isActive }) =>
                `nav-link ${isActive ? "nav-link-active" : ""}`
              }
            >
              {item.label}
            </NavLink>
          ))}
        </nav>

        <div className="flex items-center gap-2">
          <CatalogPill />
          <ConnectionPill />
          <PanicButton />
        </div>
      </header>

      <ErrorBanner />

      <main className="flex-1 overflow-auto p-5 pb-14">
        <Outlet />
      </main>

      <ThemeToggle />
    </div>
  );
}
