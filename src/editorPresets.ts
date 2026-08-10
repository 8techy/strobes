/**
 * Simple Editor patterns. Selecting one fills the step list, lamp picks, and
 * timing so the rest of the Editor reflects the choice immediately.
 */

import type { Step } from "./types";

const RING_L = 0x30;
const RING_R = 0x31;
const TFL_L = 0x03;
const TFL_R = 0x04;
const NSW_L = 0x0b;
const NSW_R = 0x0c;
const SL_L = 0x14;
const SL_R = 0x15;
const SL2_L = 0x16;
const SL2_R = 0x17;
const AL_L = 0x01;
const AL_R = 0x02;
const FL_L = 0x07;
const FL_R = 0x08;

const on = (lamp: number) => ({ lamp, level: 100 });
const off = (lamp: number) => ({ lamp, level: 0 });

/** Left bank: fog, angel eye, DRL, beams, outer + inner tails. */
const LEFT = [NSW_L, RING_L, TFL_L, AL_L, FL_L, SL_L, SL2_L] as const;
/** Right bank: same set on the other side. */
const RIGHT = [NSW_R, RING_R, TFL_R, AL_R, FL_R, SL_R, SL2_R] as const;
/** Every lamp used by the built-in patterns. */
const BOTH = [...LEFT, ...RIGHT] as const;

export type EditorPresetId =
  | "custom"
  | "strobe"
  | "left-right"
  | "double-flash"
  | "triple-flash";

export interface EditorPreset {
  id: EditorPresetId;
  label: string;
  name: string;
  looping: boolean;
  perBeat: boolean;
  steps: Step[];
}

function bank(onSide: readonly number[], offSide: readonly number[], durationMs: number): Step {
  return {
    commands: [...onSide.map(on), ...offSide.map(off)],
    duration_ms: durationMs,
  };
}

function bothOn(durationMs: number): Step {
  return {
    commands: BOTH.map(on),
    duration_ms: durationMs,
  };
}

function bothOff(durationMs: number): Step {
  return {
    commands: BOTH.map(off),
    duration_ms: durationMs,
  };
}

export const EDITOR_PRESETS: EditorPreset[] = [
  {
    id: "custom",
    label: "Custom",
    name: "My effect",
    looping: true,
    perBeat: false,
    steps: [
      { commands: [], duration_ms: 200 },
      { commands: [], duration_ms: 200 },
    ],
  },
  {
    id: "strobe",
    label: "Strobe",
    name: "Strobe",
    looping: true,
    perBeat: false,
    // JDM-style: both banks flash hard together, then a short rest.
    steps: [
      bothOn(55),
      bothOff(55),
      bothOn(55),
      bothOff(55),
      bothOn(55),
      bothOff(55),
      bothOn(55),
      bothOff(200),
    ],
  },
  {
    id: "left-right",
    label: "Left right",
    name: "Left right",
    looping: true,
    perBeat: false,
    steps: [
      bank(LEFT, RIGHT, 150),
      bank(RIGHT, LEFT, 150),
    ],
  },
  {
    id: "double-flash",
    label: "Double flash",
    name: "Double flash",
    looping: true,
    perBeat: false,
    steps: [bothOn(70), bothOff(70), bothOn(70), bothOff(350)],
  },
  {
    id: "triple-flash",
    label: "Triple flash",
    name: "Triple flash",
    looping: true,
    perBeat: false,
    steps: [
      bothOn(60),
      bothOff(60),
      bothOn(60),
      bothOff(60),
      bothOn(60),
      bothOff(350),
    ],
  },
];

export function presetById(id: EditorPresetId): EditorPreset {
  const found = EDITOR_PRESETS.find((preset) => preset.id === id);
  if (found) return found;
  return EDITOR_PRESETS[0]!;
}
