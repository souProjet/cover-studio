"use client";

import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { api, newJobId, onJobEvent } from "./api";
import type { Stage } from "./types";
import { errorMessage, isCancelled } from "./utils";

export interface JobState {
  id: string;
  title: string;
  stage: Stage | null;
  label: string;
  fraction: number | null;
  log: string[];
  status: "running" | "done" | "failed" | "cancelled";
  error?: string;
}

interface Jobs {
  current: JobState | null;
  /** Runs `fn(jobId)` while tracking its progress; rethrows failures. */
  run<T>(title: string, fn: (job: string) => Promise<T>): Promise<T>;
  cancel(): void;
  dismiss(): void;
}

const JobsContext = createContext<Jobs | null>(null);

export function JobsProvider({ children }: { children: React.ReactNode }) {
  const [current, setCurrent] = useState<JobState | null>(null);
  const ref = useRef<JobState | null>(null);
  ref.current = current;

  useEffect(() => {
    let off: (() => void) | undefined;
    onJobEvent((e) => {
      setCurrent((c) => {
        if (!c || c.id !== e.job) return c;
        if (e.type === "stage") return { ...c, stage: e.stage, label: e.label, fraction: null };
        if (e.type === "progress") return { ...c, stage: e.stage, fraction: e.fraction };
        return { ...c, log: [...c.log.slice(-30), e.message] };
      });
    }).then((f) => (off = f));
    return () => off?.();
  }, []);

  const run = useCallback(async <T,>(title: string, fn: (job: string) => Promise<T>) => {
    const id = newJobId();
    setCurrent({ id, title, stage: null, label: "", fraction: null, log: [], status: "running" });
    try {
      const r = await fn(id);
      setCurrent((c) => (c?.id === id ? { ...c, status: "done", fraction: 1 } : c));
      setTimeout(() => setCurrent((c) => (c?.id === id && c.status === "done" ? null : c)), 1800);
      return r;
    } catch (e) {
      const cancelled = isCancelled(e);
      setCurrent((c) =>
        c?.id === id ? { ...c, status: cancelled ? "cancelled" : "failed", error: errorMessage(e) } : c,
      );
      if (cancelled) setTimeout(() => setCurrent((c) => (c?.id === id ? null : c)), 1500);
      throw e;
    }
  }, []);

  const cancel = useCallback(() => {
    if (ref.current?.status === "running") api.cancelJob(ref.current.id);
  }, []);
  const dismiss = useCallback(() => setCurrent(null), []);

  return <JobsContext.Provider value={{ current, run, cancel, dismiss }}>{children}</JobsContext.Provider>;
}

export function useJobs() {
  const ctx = useContext(JobsContext);
  if (!ctx) throw new Error("useJobs outside JobsProvider");
  return ctx;
}
