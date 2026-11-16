import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { Key } from "./types";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export const NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const NOTE_NAMES_FR = ["Do", "Do#", "Ré", "Ré#", "Mi", "Fa", "Fa#", "Sol", "Sol#", "La", "La#", "Si"];

export function noteName(pc: number, lang: "fr" | "en") {
  return (lang === "fr" ? NOTE_NAMES_FR : NOTE_NAMES)[((pc % 12) + 12) % 12];
}

export function keyName(k: Key, lang: "fr" | "en") {
  const mode = lang === "fr" ? (k.mode === "major" ? "majeur" : "mineur") : k.mode;
  return `${noteName(k.tonic, lang)} ${mode}`;
}

/** m:ss.s */
export function fmtTime(sec: number, decimals = 1) {
  const s = Math.max(0, sec);
  const m = Math.floor(s / 60);
  const r = s - m * 60;
  return `${m}:${r.toFixed(decimals).padStart(decimals ? decimals + 3 : 2, "0")}`;
}

export function fmtBytes(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return `${Math.round(n / 1e3)} kB`;
}

export function basename(path: string) {
  return path.split(/[\\/]/).pop() ?? path;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function isCancelled(e: unknown) {
  return !!(e && typeof e === "object" && "cancelled" in e && (e as { cancelled: boolean }).cancelled);
}
