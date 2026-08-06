/**
 * A grid of lamp outputs showing live brightness.
 *
 * Used as both a live preview during playback and a picker in the lab and
 * editor. Safety-critical outputs are visually distinct so nobody drives the
 * brake lights by accident.
 */

import type { LampInfo } from "../types";

interface LampGridProps {
  lamps: LampInfo[];
  /** Live brightness per lamp id, 0-100. */
  levels?: Record<number, number>;
  /** Lamp ids that are selected, when used as a picker. */
  selected?: Set<number>;
  /** Lamp ids the car has recorded faults against. */
  degraded?: Set<number>;
  onPick?: (lamp: LampInfo) => void;
  /** Hide outputs that are legally regulated signalling devices. */
  featuredOnly?: boolean;
}

export function LampGrid({
  lamps,
  levels = {},
  selected,
  degraded,
  onPick,
  featuredOnly = false,
}: LampGridProps) {
  const visible = featuredOnly ? lamps.filter((lamp) => lamp.featured) : lamps;

  // Group by the label the Rust side already computed, so ordering and naming
  // stay consistent with the catalog.
  const groups = new Map<string, LampInfo[]>();
  for (const lamp of visible) {
    const existing = groups.get(lamp.group);
    if (existing) existing.push(lamp);
    else groups.set(lamp.group, [lamp]);
  }

  if (visible.length === 0) {
    return (
      <p className="text-sm text-[var(--color-ink-400)]">
        No lamps available. Load a catalog for your chassis.
      </p>
    );
  }

  return (
    <div className="space-y-4">
      {[...groups].map(([group, entries]) => (
        <div key={group}>
          <div className="label mb-2">{group}</div>
          <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-4">
            {entries.map((lamp) => {
              const level = levels[lamp.id] ?? 0;
              const isSelected = selected?.has(lamp.id) ?? false;
              const isDegraded = degraded?.has(lamp.id) ?? false;
              const interactive = Boolean(onPick) && !isDegraded;

              return (
                <button
                  key={lamp.id}
                  disabled={!interactive}
                  onClick={() => onPick?.(lamp)}
                  title={
                    isDegraded
                      ? "The car has recorded faults for this output, so Strobelight will not drive it."
                      : `${lamp.code} · ${lamp.idHex}`
                  }
                  className="relative overflow-hidden rounded-lg px-3 py-2 text-left transition-colors"
                  style={{
                    backgroundColor: "var(--color-ink-850)",
                    border: `1px solid ${
                      isSelected
                        ? "var(--color-beam-500)"
                        : isDegraded
                          ? "var(--color-danger)"
                          : "var(--color-ink-700)"
                    }`,
                    cursor: interactive ? "pointer" : "default",
                    opacity: isDegraded ? 0.55 : 1,
                  }}
                >
                  {/* Brightness is shown as a fill so a glance reads the whole
                      car's state without parsing numbers. */}
                  <span
                    aria-hidden
                    className="pointer-events-none absolute inset-0"
                    style={{
                      backgroundColor: "var(--color-beam-400)",
                      opacity: (level / 100) * 0.32,
                      transition: "opacity 60ms linear",
                    }}
                  />
                  <span className="relative block text-sm font-semibold leading-tight">
                    {lamp.name}
                  </span>
                  <span className="relative mono block text-[0.65rem] text-[var(--color-ink-400)]">
                    {lamp.code} · {lamp.idHex}
                    {lamp.safetyCritical && " · regulated"}
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}
