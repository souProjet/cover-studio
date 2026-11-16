"use client";

import { I18nProvider } from "@/lib/i18n";
import { JobsProvider } from "@/lib/jobs";
import { JobBar } from "./job-bar";

export function Providers({ children }: { children: React.ReactNode }) {
  return (
    <I18nProvider>
      <JobsProvider>
        {children}
        <JobBar />
      </JobsProvider>
    </I18nProvider>
  );
}
