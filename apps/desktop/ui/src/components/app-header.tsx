"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import { useI18n } from "@/lib/i18n";
import { cn } from "@/lib/utils";

export function Wordmark() {
  return (
    <span className="flex items-center gap-2.5">
      <span aria-hidden className="flex h-7 items-end gap-[3px]">
        {[10, 18, 26, 20, 13, 7].map((h, i) => (
          <span key={i} className="w-[3px] rounded-full bg-amber" style={{ height: h }} />
        ))}
      </span>
      <span className="font-display text-[19px] font-extrabold tracking-[-0.02em]">Cover Studio</span>
    </span>
  );
}

export function AppHeader({ children }: { children?: React.ReactNode }) {
  const { t, lang, setLang } = useI18n();
  const path = usePathname();
  const nav = [
    { href: "/", label: t("nav.projects") },
    { href: "/settings/", label: t("nav.settings") },
  ];
  return (
    <header className="sticky top-0 z-40 flex h-14 items-center gap-6 border-b border-line bg-console/95 px-6 backdrop-blur">
      <Link href="/" className="shrink-0">
        <Wordmark />
      </Link>
      <div className="flex min-w-0 flex-1 items-center gap-4">{children}</div>
      <nav className="flex items-center gap-1 text-sm">
        {nav.map((n) => (
          <Link
            key={n.href}
            href={n.href}
            className={cn(
              "rounded-md px-3 py-1.5 text-muted transition-colors hover:text-ink",
              (path === n.href || (n.href !== "/" && path.startsWith(n.href))) && "text-ink",
            )}
          >
            {n.label}
          </Link>
        ))}
        <button
          type="button"
          onClick={() => setLang(lang === "fr" ? "en" : "fr")}
          className="ml-2 cursor-pointer rounded-md border border-line px-2 py-1 font-mono text-[11px] uppercase text-muted hover:text-ink"
          aria-label={t("settings.language")}
        >
          {lang === "fr" ? "en" : "fr"}
        </button>
      </nav>
    </header>
  );
}
