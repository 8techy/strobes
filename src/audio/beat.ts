/**
 * Beat detection from a microphone or system audio input.
 *
 * Runs in the frontend because that is where Web Audio lives, but it only
 * produces *events*: the actual timing of light changes stays in Rust, since
 * JavaScript timer jitter would be a visible fraction of a 20 ms effect step.
 *
 * The algorithm is a low-band energy comparison against a rolling average, which
 * is simple, cheap and good enough for dance music. It is not trying to be a
 * proper onset detector.
 */

export interface BeatDetectorOptions {
  /** Called on each detected beat with the current BPM estimate. */
  onBeat: (bpm: number) => void;
  /** Called with a 0-1 level for the meter. */
  onLevel?: (level: number) => void;
  /** How much louder than the rolling average a beat must be. */
  sensitivity?: number;
}

/** Ignore beats closer together than this; 300 ms caps out at 200 BPM. */
const MIN_BEAT_INTERVAL_MS = 300;
/** Only the low end drives beat detection: kick drums live here. */
const LOW_BAND_HZ = 200;
/** How many intervals to average the BPM estimate over. */
const BPM_WINDOW = 8;

export class BeatDetector {
  private context?: AudioContext;
  private analyser?: AnalyserNode;
  private stream?: MediaStream;
  private frame?: number;

  private energyHistory: number[] = [];
  private intervals: number[] = [];
  private lastBeatAt = 0;
  private readonly sensitivity: number;

  constructor(private readonly options: BeatDetectorOptions) {
    this.sensitivity = options.sensitivity ?? 1.35;
  }

  /** Requests audio input and begins analysis. */
  async start(): Promise<void> {
    this.stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        // Every one of these would fight the detector by flattening dynamics.
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
    });

    this.context = new AudioContext();
    const source = this.context.createMediaStreamSource(this.stream);
    this.analyser = this.context.createAnalyser();
    this.analyser.fftSize = 1024;
    this.analyser.smoothingTimeConstant = 0.2;
    source.connect(this.analyser);

    this.loop();
  }

  stop(): void {
    if (this.frame !== undefined) cancelAnimationFrame(this.frame);
    this.frame = undefined;
    for (const track of this.stream?.getTracks() ?? []) track.stop();
    void this.context?.close();
    this.context = undefined;
    this.analyser = undefined;
    this.stream = undefined;
    this.energyHistory = [];
    this.intervals = [];
    this.lastBeatAt = 0;
  }

  /** Current BPM estimate, or null before enough beats have landed. */
  get bpm(): number | null {
    if (this.intervals.length < 2) return null;
    const mean =
      this.intervals.reduce((sum, value) => sum + value, 0) / this.intervals.length;
    return mean > 0 ? 60000 / mean : null;
  }

  private loop = (): void => {
    const analyser = this.analyser;
    const context = this.context;
    if (!analyser || !context) return;

    const bins = new Uint8Array(analyser.frequencyBinCount);
    analyser.getByteFrequencyData(bins);

    // Only sum bins below LOW_BAND_HZ.
    const hzPerBin = context.sampleRate / 2 / analyser.frequencyBinCount;
    const lowBinCount = Math.max(1, Math.floor(LOW_BAND_HZ / hzPerBin));
    let energy = 0;
    for (let i = 0; i < lowBinCount; i += 1) {
      const value = (bins[i] ?? 0) / 255;
      energy += value * value;
    }
    energy /= lowBinCount;

    this.options.onLevel?.(Math.min(1, Math.sqrt(energy) * 1.6));

    // Compare against the rolling average, then push. Comparing before pushing
    // stops a loud beat from raising the very threshold it must clear.
    const average =
      this.energyHistory.length > 0
        ? this.energyHistory.reduce((sum, value) => sum + value, 0) /
          this.energyHistory.length
        : 0;

    this.energyHistory.push(energy);
    if (this.energyHistory.length > 43) this.energyHistory.shift();

    const now = performance.now();
    const loudEnough = energy > average * this.sensitivity && energy > 0.008;
    const settled = now - this.lastBeatAt > MIN_BEAT_INTERVAL_MS;

    if (loudEnough && settled && this.energyHistory.length > 10) {
      if (this.lastBeatAt > 0) {
        this.intervals.push(now - this.lastBeatAt);
        if (this.intervals.length > BPM_WINDOW) this.intervals.shift();
      }
      this.lastBeatAt = now;
      this.options.onBeat(this.bpm ?? 0);
    }

    this.frame = requestAnimationFrame(this.loop);
  };
}
