"use client";

import { Check, Copy, Crosshair } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { InstrumentKind, Mode, Project, RenderInfo, Settings, SoundPreset, VocalSettings } from "@/lib/types";
import { cn, fmtTime, keyName, noteName } from "@/lib/utils";
import { Button, Fader, Label, SectionTitle, Segmented, Select, Switch, TextInput } from "../ui";

type Tab = "sound" | "tuning" | "video" | "post" | "advanced";

const PRESET_VOCAL: Record<SoundPreset, VocalSettings> = {
  natural: {
    blend: 0.6,
    presence_db: 0.8,
    air_db: 0.5,
    de_ess: 0.4,
    compression: 0.3,
    leveler: 0.35,
    breath: 0.25,
    cleanup: 0.3,
    harshness: 0.3,
    reverb: 0.1,
  },
  balanced: {
    blend: 0.8,
    presence_db: 1.5,
    air_db: 1,
    de_ess: 0.55,
    compression: 0.5,
    leveler: 0.55,
    breath: 0.4,
    cleanup: 0.45,
    harshness: 0.5,
    reverb: 0.18,
  },
  polished: {
    blend: 0.95,
    presence_db: 2.3,
    air_db: 1.6,
    de_ess: 0.7,
    compression: 0.65,
    leveler: 0.75,
    breath: 0.55,
    cleanup: 0.6,
    harshness: 0.65,
    reverb: 0.26,
  },
};
// [strength, retune speed, drift, vibrato], instrument blend, warmth, room
const PRESET_REST: Record<SoundPreset, [[number, number, number, number], number, number, number]> = {
  natural: [[0.6, 0.15, 0.15, 0], 0.4, 0.6, 0.15],
  balanced: [[0.85, 0.35, 0.35, 0.2], 0.6, 0.8, 0.22],
  polished: [[0.95, 0.6, 0.6, 0.4], 0.65, 0.7, 0.25],
};

/** Mirrors `Settings::apply_preset` in the engine. */
function applyPreset(s: Settings, p: SoundPreset): Settings {
  const [[strength, retune_speed, drift, vibrato], instrument_blend, warmth_db, room] = PRESET_REST[p];
  return {
    ...s,
    preset: p,
    vocal: { ...PRESET_VOCAL[p] },
    tuning: { ...s.tuning, strength, retune_speed, drift, vibrato },
    instrument_blend,
    warmth_db,
    room,
  };
}

const pct = (v: number) => `${Math.round(v * 100)}%`;
const db = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;

function NotesMap({ render }: { render: RenderInfo }) {
  const notes = render.notes;
  if (!notes.length) return null;
  const lo = Math.floor(Math.min(...notes.map((n) => Math.min(n.sung_midi, n.target_midi)))) - 1;
  const hi = Math.ceil(Math.max(...notes.map((n) => Math.max(n.sung_midi, n.target_midi)))) + 1;
  const len = render.end_sec - render.start_sec;
  const x = (t: number) => ((t - render.start_sec) / len) * 100;
  const y = (m: number) => ((hi - m) / (hi - lo)) * 100;
  return (
    <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-36 w-full rounded-lg border border-line bg-console">
      {Array.from({ length: hi - lo }, (_, i) => (
        <line key={i} x1="0" x2="100" y1={y(lo + i)} y2={y(lo + i)} stroke="#2f4148" strokeWidth="0.3" vectorEffect="non-scaling-stroke" />
      ))}
      {notes.map((n, i) => (
        <g key={i}>
          <line x1={x(n.start_sec)} x2={x(n.end_sec)} y1={y(n.target_midi)} y2={y(n.target_midi)} stroke="#f2a541" strokeWidth="3" vectorEffect="non-scaling-stroke" />
          <line x1={x(n.start_sec)} x2={x(n.end_sec)} y1={y(n.sung_midi)} y2={y(n.sung_midi)} stroke="#7fb7be" strokeWidth="2" strokeOpacity="0.8" vectorEffect="non-scaling-stroke" />
        </g>
      ))}
    </svg>
  );
}

function SyncPanel({ project, update }: { project: Project; update: (s: Settings) => void }) {
  const { t } = useI18n();
  const s = project.settings;
  const sync = project.analysis?.sync;
  if (!sync) return <p className="text-sm text-faint">{t("sync.none")}</p>;
  const offset = s.sync_offset_sec ?? sync.offset_sec;
  const manual = s.sync_offset_sec !== null;
  const nudge = (d: number) => update({ ...s, sync_offset_sec: Math.round((offset + d) * 1000) / 1000 });
  return (
    <div className="grid gap-3">
      <div className="flex items-baseline justify-between">
        <Label>{t("sync.offset")}</Label>
        <span className="font-mono text-sm tabular-nums">
          {offset >= 0 ? "+" : ""}
          {offset.toFixed(3)} s <span className="text-xs text-faint">· {t(manual ? "sync.manual" : "sync.detected")}</span>
        </span>
      </div>
      <div className="grid grid-cols-4 gap-1">
        {[-0.1, -0.01, 0.01, 0.1].map((d) => (
          <Button key={d} size="sm" variant="secondary" className="font-mono text-xs" onClick={() => nudge(d)}>
            {d > 0 ? "+" : "−"}
            {Math.abs(d * 1000)} ms
          </Button>
        ))}
      </div>
      <div className="flex items-center justify-between text-xs text-faint">
        <span>
          {Math.round(sync.confidence * 100)}%{sync.method !== "correlation" && ` · ${t("sync.clap")}`}
        </span>
        {manual && (
          <button type="button" className="cursor-pointer text-muted underline-offset-2 hover:underline" onClick={() => update({ ...s, sync_offset_sec: null })}>
            {t("sync.reset")}
          </button>
        )}
      </div>
      {sync.confidence < 0.5 && !manual && <p className="text-xs text-amber">{t("sync.weak")}</p>}
    </div>
  );
}

function Caption({ project }: { project: Project }) {
  const { t } = useI18n();
  const [text, setText] = useState("");
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    const id = setTimeout(() => api.captionPreview(project.id).then(setText).catch(() => {}), 450);
    return () => clearTimeout(id);
  }, [project.id, project.settings.social, project.settings.instrument]);
  return (
    <div className="grid gap-1">
      <div className="flex items-center justify-between">
        <Label>{t("post.caption")}</Label>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => {
            navigator.clipboard.writeText(text);
            setCopied(true);
            setTimeout(() => setCopied(false), 1500);
          }}
        >
          {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
          {t(copied ? "post.copied" : "post.copy")}
        </Button>
      </div>
      <pre className="max-h-48 overflow-auto rounded-lg border border-line bg-console p-3 font-sans text-sm whitespace-pre-wrap text-ink/90 select-text">
        {text}
      </pre>
    </div>
  );
}

export function Controls({
  project,
  update,
  playhead,
}: {
  project: Project;
  update: (s: Settings) => void;
  playhead: number | null;
}) {
  const { t, lang } = useI18n();
  const [tab, setTab] = useState<Tab>("sound");
  const s = project.settings;
  const set = <K extends keyof Settings>(k: K, v: Settings[K]) => update({ ...s, [k]: v });
  const key = project.analysis?.key;

  const tabs: { id: Tab; label: string }[] = [
    { id: "sound", label: t("tab.sound") },
    { id: "tuning", label: t("tab.tuning") },
    { id: "video", label: t("tab.video") },
    { id: "post", label: t("tab.post") },
    { id: "advanced", label: t("tab.advanced") },
  ];

  const instruments: InstrumentKind[] = [
    "acoustic_guitar",
    "classical_guitar",
    "electric_guitar",
    "piano",
    "ukulele",
    "full_band",
    "other",
  ];

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div role="tablist" className="flex gap-1 border-b border-line px-2">
        {tabs.map((x) => (
          <button
            key={x.id}
            role="tab"
            type="button"
            aria-selected={tab === x.id}
            onClick={() => setTab(x.id)}
            className={cn(
              "-mb-px cursor-pointer border-b-2 px-3 py-2.5 text-sm transition-colors",
              tab === x.id ? "border-amber text-ink" : "border-transparent text-muted hover:text-ink",
            )}
          >
            {x.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto p-5">
        {tab === "sound" && (
          <div className="grid gap-6">
            <div className="grid gap-2">
              <SectionTitle>{t("sound.preset")}</SectionTitle>
              <Segmented
                value={s.preset}
                onChange={(p) => update(applyPreset(s, p))}
                options={(["natural", "balanced", "polished"] as const).map((p) => ({
                  value: p,
                  label: t(`preset.${p}`),
                  hint: t(`preset.${p}.hint`),
                }))}
              />
              <p className="text-xs text-faint">{t(`preset.${s.preset}.hint`)}</p>
            </div>
            <Select
              label={t("sound.instrument")}
              value={s.instrument}
              onChange={(v) => set("instrument", v)}
              options={instruments.map((i) => ({ value: i, label: t(`instrument.${i}`) }))}
            />
            <div className="grid gap-4">
              <SectionTitle>{t("sound.voice")}</SectionTitle>
              <Fader label={t("sound.voiceAmount")} value={s.vocal.blend} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, blend: v })} />
              <Fader label={t("sound.presence")} value={s.vocal.presence_db} min={-3} max={5} step={0.1} format={db} onChange={(v) => set("vocal", { ...s.vocal, presence_db: v })} />
              <Fader label={t("sound.air")} value={s.vocal.air_db} min={-3} max={4} step={0.1} format={db} onChange={(v) => set("vocal", { ...s.vocal, air_db: v })} />
              <Fader label={t("sound.leveler")} value={s.vocal.leveler} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, leveler: v })} />
              <Fader label={t("sound.compression")} value={s.vocal.compression} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, compression: v })} />
              <Fader label={t("sound.deess")} value={s.vocal.de_ess} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, de_ess: v })} />
              <Fader label={t("sound.breath")} value={s.vocal.breath} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, breath: v })} />
              <Fader label={t("sound.vocalReverb")} value={s.vocal.reverb} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, reverb: v })} />
            </div>
            <div className="grid gap-4">
              <SectionTitle>{t("sound.mix")}</SectionTitle>
              <Fader label={t("sound.instrumentAmount")} value={s.instrument_blend} min={0} max={1} format={pct} onChange={(v) => set("instrument_blend", v)} />
              <Fader label={t("sound.warmth")} value={s.warmth_db} min={-2} max={3} step={0.1} format={db} onChange={(v) => set("warmth_db", v)} />
              <Fader label={t("sound.room")} value={s.room} min={0} max={1} format={pct} onChange={(v) => set("room", v)} />
              <Select
                label={t("sound.loudness")}
                value={String(s.loudness.lufs)}
                onChange={(v) => set("loudness", { ...s.loudness, lufs: Number(v) })}
                options={[
                  { value: "-14", label: t("loudness.streaming") },
                  { value: "-16", label: t("loudness.quiet") },
                  { value: "-11", label: t("loudness.loud") },
                ]}
              />
            </div>
          </div>
        )}

        {tab === "tuning" && (
          <div className="grid gap-5">
            <Switch label={t("tuning.enabled")} checked={s.tuning.enabled} onChange={(v) => set("tuning", { ...s.tuning, enabled: v })} />
            <Fader label={t("tuning.strength")} value={s.tuning.strength} min={0} max={1} format={pct} disabled={!s.tuning.enabled} onChange={(v) => set("tuning", { ...s.tuning, strength: v })} />
            <div className="grid gap-1">
              <Fader label={t("tuning.retune")} value={s.tuning.retune_speed} min={0} max={1} format={pct} disabled={!s.tuning.enabled} onChange={(v) => set("tuning", { ...s.tuning, retune_speed: v })} />
              <p className="text-xs text-faint">{t("tuning.retuneHint")}</p>
            </div>
            <Fader label={t("tuning.drift")} value={s.tuning.drift} min={0} max={1} format={pct} disabled={!s.tuning.enabled} onChange={(v) => set("tuning", { ...s.tuning, drift: v })} />
            <Fader label={t("tuning.vibrato")} value={s.tuning.vibrato} min={0} max={1} format={pct} disabled={!s.tuning.enabled} onChange={(v) => set("tuning", { ...s.tuning, vibrato: v })} />
            <div className="grid gap-2">
              <SectionTitle>{t("tuning.scale")}</SectionTitle>
              <Segmented
                value={s.tuning.scale.type}
                onChange={(type) =>
                  set("tuning", {
                    ...s.tuning,
                    scale: type === "manual" ? { type, tonic: key?.tonic ?? 0, mode: key?.mode ?? "major" } : { type },
                  })
                }
                options={[
                  { value: "auto", label: key ? keyName(key, lang) : t("tuning.auto") },
                  { value: "chromatic", label: t("tuning.chromatic") },
                  { value: "manual", label: t("tuning.manual") },
                ]}
              />
              {s.tuning.scale.type === "manual" && (
                <div className="grid grid-cols-2 gap-2">
                  <Select
                    value={String(s.tuning.scale.tonic)}
                    onChange={(v) => s.tuning.scale.type === "manual" && set("tuning", { ...s.tuning, scale: { ...s.tuning.scale, tonic: Number(v) } })}
                    options={Array.from({ length: 12 }, (_, i) => ({ value: String(i), label: noteName(i, lang) }))}
                  />
                  <Select<Mode>
                    value={s.tuning.scale.mode}
                    onChange={(v) => s.tuning.scale.type === "manual" && set("tuning", { ...s.tuning, scale: { ...s.tuning.scale, mode: v } })}
                    options={[
                      { value: "major", label: t("tuning.major") },
                      { value: "minor", label: t("tuning.minor") },
                    ]}
                  />
                </div>
              )}
            </div>
            <p className="text-xs leading-relaxed text-faint">{t("tuning.hint")}</p>
            {project.last_render && <NotesMap render={project.last_render} />}
          </div>
        )}

        {tab === "video" && (
          <div className="grid gap-6">
            {!project.sources.video && <p className="text-sm text-faint">{t("video.none")}</p>}
            {project.sources.audio && project.sources.video && (
              <div className="grid gap-2">
                <SectionTitle>{t("sync.title")}</SectionTitle>
                <SyncPanel project={project} update={update} />
              </div>
            )}
            <div className="grid gap-2">
              <SectionTitle>{t("video.format")}</SectionTitle>
              <Segmented
                value={s.video.aspect}
                onChange={(v) => set("video", { ...s.video, aspect: v })}
                options={(["vertical", "square", "landscape", "source"] as const).map((a) => ({
                  value: a,
                  label: { vertical: "9:16", square: "1:1", landscape: "16:9", source: "Source" }[a],
                  hint: t(`aspect.${a}`),
                }))}
              />
              <p className="text-xs text-faint">{t(`aspect.${s.video.aspect}`)}</p>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <Select
                label={t("video.style")}
                value={s.video.style}
                onChange={(v) => set("video", { ...s.video, style: v })}
                options={(["natural", "social", "low_light"] as const).map((v) => ({ value: v, label: t(`style.${v}`) }))}
              />
              <Select
                label={t("video.quality")}
                value={s.video.quality}
                onChange={(v) => set("video", { ...s.video, quality: v })}
                options={(["draft", "high", "max"] as const).map((v) => ({ value: v, label: t(`quality.${v}`) }))}
              />
            </div>
            <Fader label={t("video.fadeIn")} value={s.video.fade_in_sec} min={0} max={3} step={0.1} format={(v) => `${v.toFixed(1)} s`} onChange={(v) => set("video", { ...s.video, fade_in_sec: v })} />
            <Fader label={t("video.fadeOut")} value={s.video.fade_out_sec} min={0} max={4} step={0.1} format={(v) => `${v.toFixed(1)} s`} onChange={(v) => set("video", { ...s.video, fade_out_sec: v })} />
          </div>
        )}

        {tab === "post" && (
          <div className="grid gap-5">
            <TextInput label={t("post.title")} value={s.social.title} onChange={(e) => set("social", { ...s.social, title: e.target.value })} />
            <TextInput label={t("post.artist")} value={s.social.artist} onChange={(e) => set("social", { ...s.social, artist: e.target.value })} />
            <div className="grid gap-2">
              <Label>{t("post.language")}</Label>
              <Segmented
                value={s.social.language}
                onChange={(v) => set("social", { ...s.social, language: v })}
                options={[
                  { value: "fr", label: "Français" },
                  { value: "en", label: "English" },
                ]}
              />
            </div>
            <TextInput
              label={t("post.hashtags")}
              hint={t("post.hashtagsHint")}
              defaultValue={s.social.extra_hashtags.join(" ")}
              onBlur={(e) =>
                set("social", { ...s.social, extra_hashtags: e.target.value.split(/[\s,]+/).filter(Boolean) })
              }
            />
            <div className="grid gap-2">
              <Label>{t("post.thumbnail")}</Label>
              <div className="flex items-center gap-2">
                <Button size="sm" disabled={playhead === null} onClick={() => playhead !== null && set("social", { ...s.social, thumbnail_at_sec: playhead })}>
                  <Crosshair className="h-3.5 w-3.5" />
                  {t("post.thumbnailAt")}
                </Button>
                <span className="font-mono text-xs text-muted">
                  {s.social.thumbnail_at_sec !== null ? fmtTime(s.social.thumbnail_at_sec) : t("post.thumbnailAuto")}
                </span>
                {s.social.thumbnail_at_sec !== null && (
                  <Button size="sm" variant="ghost" onClick={() => set("social", { ...s.social, thumbnail_at_sec: null })}>
                    {t("post.thumbnailAuto")}
                  </Button>
                )}
              </div>
            </div>
            <Caption project={project} />
          </div>
        )}

        {tab === "advanced" && (
          <div className="grid gap-5">
            <Select
              label={t("adv.model")}
              value={s.separation_model}
              onChange={(v) => set("separation_model", v)}
              options={[
                { value: "mdx-voc-ft", label: "MDX-Net Voc FT" },
                { value: "mdx-inst-hq3", label: "MDX-Net Inst HQ 3" },
              ]}
            />
            <Switch label={t("adv.denoise")} checked={s.denoise_separation} onChange={(v) => set("denoise_separation", v)} />
            <Select
              label={t("adv.device")}
              value={s.device}
              onChange={(v) => set("device", v)}
              options={(["auto", "cuda", "cpu"] as const).map((d) => ({ value: d, label: t(`device.${d}`) }))}
            />
            <p className="text-xs text-faint">{t("adv.reanalyzeHint")}</p>
            <div className="grid gap-4">
              <SectionTitle>{t("adv.voice")}</SectionTitle>
              <Fader label={t("adv.cleanup")} value={s.vocal.cleanup} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, cleanup: v })} />
              <Fader label={t("adv.harshness")} value={s.vocal.harshness} min={0} max={1} format={pct} onChange={(v) => set("vocal", { ...s.vocal, harshness: v })} />
              <Switch label={t("adv.formants")} checked={s.tuning.preserve_formants} onChange={(v) => set("tuning", { ...s.tuning, preserve_formants: v })} />
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
