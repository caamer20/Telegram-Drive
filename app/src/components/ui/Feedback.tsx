import type { HTMLAttributes } from 'react';
import { cx } from './cx';

interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger';
}

const badgeTones = {
  neutral: 'border-app-border bg-app-hover text-app-text-secondary',
  accent: 'border-app-accent/20 bg-app-selected text-app-text',
  success: 'border-app-success/20 bg-app-success/10 text-app-text',
  warning: 'border-app-warning/20 bg-app-warning/10 text-app-text',
  danger: 'border-app-danger/20 bg-app-danger/10 text-app-text',
};

export function Badge({ tone = 'neutral', className, ...props }: BadgeProps) {
  return <span className={cx('inline-flex items-center rounded-full border px-2 py-0.5 text-badge font-medium', badgeTones[tone], className)} {...props} />;
}

export function StatusDot({ tone = 'neutral', label }: { tone?: 'neutral' | 'success' | 'warning' | 'danger'; label: string }) {
  const tones = { neutral: 'bg-app-text-tertiary', success: 'bg-app-success', warning: 'bg-app-warning', danger: 'bg-app-danger' };
  return <span className={cx('inline-block h-2 w-2 rounded-full', tones[tone])} role="img" aria-label={label} title={label} />;
}

export function Progress({ value, label, className }: { value: number; label: string; className?: string }) {
  const bounded = Math.min(100, Math.max(0, value));
  return (
    <div className={cx('h-1.5 overflow-hidden rounded-full bg-app-border', className)} role="progressbar" aria-label={label} aria-valuenow={bounded} aria-valuemin={0} aria-valuemax={100}>
      <div className="h-full rounded-full bg-app-accent transition-[width] duration-120 ease-linear" style={{ width: `${bounded}%` }} />
    </div>
  );
}

export function Divider({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cx('h-px bg-app-border-subtle', className)} {...props} />;
}

export function Skeleton({ className, ...props }: HTMLAttributes<HTMLDivElement>) {
  return <div aria-hidden="true" className={cx('animate-pulse rounded-control bg-app-hover', className)} {...props} />;
}
