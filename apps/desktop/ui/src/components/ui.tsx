"use client";

import { cva, type VariantProps } from "class-variance-authority";
import { useId } from "react";
import { cn } from "@/lib/utils";

const button = cva(
  "inline-flex items-center justify-center gap-2 rounded-lg font-medium transition-colors disabled:pointer-events-none disabled:opacity-40 cursor-pointer whitespace-nowrap",
  {
    variants: {
      variant: {
        primary: "bg-amber text-[#2a1a05] hover:bg-[#f7b65f] shadow-[inset_0_-2px_0_rgb(0_0_0/0.18)]",
        secondary: "bg-raised text-ink border border-line hover:border-muted",
        ghost: "text-muted hover:text-ink hover:bg-raised",
        danger: "text-danger hover:bg-danger/10",
      },
      size: {
        sm: "h-8 px-3 text-sm",
        md: "h-10 px-4 text-sm",
        lg: "h-12 px-6 text-base",
        icon: "h-9 w-9",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
);

export function Button({
  className,
  variant,
  size,
  ...props
}: React.ButtonHTMLAttributes<HTMLButtonElement> & VariantProps<typeof button>) {
  return <button className={cn(button({ variant, size }), className)} {...props} />;
}

export function Label({ className, ...props }: React.LabelHTMLAttributes<HTMLLabelElement>) {
  return <label className={cn("text-[13px] font-medium text-muted", className)} {...props} />;
}

export function SectionTitle({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <h3 className={cn("font-mono text-[11px] uppercase tracking-[0.14em] text-faint", className)}>{children}</h3>
  );
}

/** Console fader: label, value readout and a range input. */
export function Fader({
  label,
  value,
  min,
  max,
  step = 0.01,
  format,
  onChange,
  disabled,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  format?: (v: number) => string;
  onChange: (v: number) => void;
  disabled?: boolean;
}) {
  const id = useId();
  const fill = ((value - min) / (max - min)) * 100;
  return (
    <div className={cn("grid gap-1", disabled && "opacity-40")}>
      <div className="flex items-baseline justify-between">
        <Label htmlFor={id}>{label}</Label>
        <span className="font-mono text-xs text-ink/80 tabular-nums">{format ? format(value) : value}</span>
      </div>
      <input
        id={id}
        type="range"
        className="fader w-full"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        style={{ "--fill": `${fill}%` } as React.CSSProperties}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className="group flex w-full cursor-pointer items-center justify-between gap-3 py-1 text-left disabled:opacity-40"
    >
      <span className="text-sm text-ink">{label}</span>
      <span
        className={cn(
          "relative h-5 w-9 shrink-0 rounded-full border transition-colors",
          checked ? "border-amber-deep bg-amber/90" : "border-line bg-console",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 h-3.5 w-3.5 rounded-full bg-ink shadow transition-all",
            checked ? "left-[18px]" : "left-0.5",
          )}
        />
      </span>
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  className,
}: {
  value: T;
  options: { value: T; label: string; hint?: string }[];
  onChange: (v: T) => void;
  className?: string;
}) {
  return (
    <div className={cn("grid auto-cols-fr grid-flow-col gap-1 rounded-xl border border-line bg-console p-1", className)}>
      {options.map((o) => (
        <button
          key={o.value}
          type="button"
          title={o.hint}
          aria-pressed={o.value === value}
          onClick={() => onChange(o.value)}
          className={cn(
            "cursor-pointer rounded-lg px-2 py-1.5 text-sm transition-colors",
            o.value === value ? "bg-raised text-ink shadow-[inset_0_-2px_0_var(--color-amber)]" : "text-muted hover:text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label?: string;
}) {
  const id = useId();
  return (
    <div className="grid gap-1">
      {label && <Label htmlFor={id}>{label}</Label>}
      <select
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
        className="h-9 cursor-pointer rounded-lg border border-line bg-console px-3 text-sm text-ink outline-none hover:border-muted"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>
            {o.label}
          </option>
        ))}
      </select>
    </div>
  );
}

export function TextInput({
  label,
  hint,
  className,
  ...props
}: React.InputHTMLAttributes<HTMLInputElement> & { label: string; hint?: string }) {
  const id = useId();
  return (
    <div className={cn("grid gap-1", className)}>
      <Label htmlFor={id}>{label}</Label>
      <input
        id={id}
        className="h-9 rounded-lg border border-line bg-console px-3 text-sm text-ink outline-none placeholder:text-faint hover:border-muted focus:border-amber"
        {...props}
      />
      {hint && <span className="text-xs text-faint">{hint}</span>}
    </div>
  );
}

export function Progress({ value }: { value: number | null }) {
  return (
    <div className="relative h-1.5 w-full overflow-hidden rounded-full bg-console">
      {value === null ? (
        <div className="absolute inset-y-0 w-1/3 animate-[slide_1.2s_ease-in-out_infinite] rounded-full bg-amber/70" />
      ) : (
        <div className="h-full rounded-full bg-amber transition-[width] duration-200" style={{ width: `${value * 100}%` }} />
      )}
      <style>{`@keyframes slide{0%{left:-33%}100%{left:100%}}`}</style>
    </div>
  );
}
