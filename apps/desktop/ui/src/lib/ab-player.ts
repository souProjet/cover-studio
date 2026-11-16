export type Side = "a" | "b";

export interface PlayerState {
  time: number;
  playing: boolean;
}

/** A/B preview: "a" is the processed version, "b" the level-matched original. */
export interface PreviewPlayer {
  play(): Promise<PlayerState>;
  pause(): Promise<PlayerState>;
  seek(sec: number): Promise<PlayerState>;
  setSide(side: Side): void;
  state(): Promise<PlayerState>;
  dispose(): void;
}

/** Playback in the Rust shell: the webview's media stack is unreliable on Linux. */
export class NativePlayer implements PreviewPlayer {
  private constructor(private invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>) {}

  static async load(projectId: string): Promise<NativePlayer> {
    const { invoke } = await import("@tauri-apps/api/core");
    await invoke("preview_load", { id: projectId });
    return new NativePlayer(invoke);
  }

  private control(args: { playing?: boolean; seek?: number; original?: boolean }) {
    return this.invoke<PlayerState>("preview_control", args);
  }

  play() {
    return this.control({ playing: true });
  }
  pause() {
    return this.control({ playing: false });
  }
  seek(sec: number) {
    return this.control({ seek: sec });
  }
  setSide(side: Side) {
    void this.control({ original: side === "b" });
  }
  state() {
    return this.control({});
  }
  dispose() {
    void this.invoke("preview_unload");
  }
}

const FADE_SEC = 0.03;

/**
 * Browser fallback: two decoded versions started on one clock, so switching
 * between them is a short gain crossfade with no gap or drift.
 */
export class AbPlayer implements PreviewPlayer {
  private ctx: AudioContext;
  private buffers: [AudioBuffer, AudioBuffer];
  private gains: [GainNode, GainNode];
  private sources: AudioBufferSourceNode[] = [];
  private side: Side = "a";
  private startedAt = 0;
  private offset = 0;

  private constructor(ctx: AudioContext, buffers: [AudioBuffer, AudioBuffer]) {
    this.ctx = ctx;
    this.buffers = buffers;
    this.gains = [ctx.createGain(), ctx.createGain()];
    for (const g of this.gains) g.connect(ctx.destination);
    this.gains[1].gain.value = 0;
  }

  static async load(a: ArrayBuffer, b: ArrayBuffer): Promise<AbPlayer> {
    const ctx = new AudioContext();
    try {
      const buffers = await Promise.all([ctx.decodeAudioData(a), ctx.decodeAudioData(b)]);
      return new AbPlayer(ctx, buffers as [AudioBuffer, AudioBuffer]);
    } catch (e) {
      void ctx.close();
      throw e;
    }
  }

  private get duration() {
    return Math.min(this.buffers[0].duration, this.buffers[1].duration);
  }

  private get playing() {
    return this.sources.length > 0;
  }

  private get time() {
    if (!this.playing) return this.offset;
    return Math.min(this.offset + this.ctx.currentTime - this.startedAt, this.duration);
  }

  private snapshot(): PlayerState {
    return { time: this.time, playing: this.playing };
  }

  async state() {
    return this.snapshot();
  }

  async play() {
    if (!this.playing) await this.start();
    return this.snapshot();
  }

  private async start() {
    await this.ctx.resume();
    if (this.offset >= this.duration - 0.01) this.offset = 0;
    const when = this.ctx.currentTime + 0.02;
    this.sources = this.buffers.map((buf, i) => {
      const src = this.ctx.createBufferSource();
      src.buffer = buf;
      src.connect(this.gains[i]);
      src.start(when, this.offset);
      return src;
    });
    this.startedAt = when;
    this.sources[0].onended = () => {
      if (!this.sources.length || this.time < this.duration - 0.05) return;
      this.stopSources();
      this.offset = this.duration;
    };
  }

  async pause() {
    if (this.playing) {
      this.offset = this.time;
      this.stopSources();
    }
    return this.snapshot();
  }

  async seek(sec: number) {
    const resume = this.playing;
    this.stopSources();
    this.offset = Math.max(0, Math.min(sec, this.duration));
    if (resume) await this.start();
    return this.snapshot();
  }

  setSide(side: Side) {
    if (side === this.side) return;
    this.side = side;
    const now = this.ctx.currentTime;
    this.gains.forEach((g, i) => {
      const target = (i === 0) === (side === "a") ? 1 : 0;
      g.gain.cancelScheduledValues(now);
      if (!this.playing) {
        g.gain.value = target;
        return;
      }
      g.gain.setValueAtTime(g.gain.value, now);
      g.gain.linearRampToValueAtTime(target, now + FADE_SEC);
    });
  }

  dispose() {
    this.stopSources();
    void this.ctx.close();
  }

  private stopSources() {
    const srcs = this.sources;
    this.sources = [];
    for (const s of srcs) {
      s.onended = null;
      try {
        s.stop();
      } catch {
        // already stopped
      }
      s.disconnect();
    }
  }
}
