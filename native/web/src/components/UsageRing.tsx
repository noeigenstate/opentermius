import { t } from "../i18n";

const RADIUS = 17;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** Usage ring: blue below 70 %, amber below 90 %, red above; "--" while unknown. */
export function UsageRing({ value, label }: { value: number | null; label: string }) {
  const percentage = value === null || !Number.isFinite(value) ? null : Math.max(0, Math.min(value, 100));
  const tone = percentage === null ? "var(--muted)" : percentage >= 90 ? "var(--danger)" : percentage >= 70 ? "var(--warning)" : "var(--accent)";
  const filled = percentage === null ? 0 : (percentage / 100) * CIRCUMFERENCE;
  return (
    <div className="ring" role="img" aria-label={percentage === null ? t("{label}: not available", { label }) : t("{label}: {value}%", { label, value: Math.round(percentage) })}>
      <svg width={42} height={42} viewBox="0 0 42 42" aria-hidden="true">
        <circle cx={21} cy={21} r={RADIUS} fill="none" stroke="var(--ring-track)" strokeWidth={3} />
        <circle className="usage-ring-progress" cx={21} cy={21} r={RADIUS} fill="none" stroke={tone} strokeWidth={3} strokeLinecap="round"
          strokeDasharray={`${filled} ${CIRCUMFERENCE}`} opacity={filled > 0 ? 1 : 0} transform="rotate(-90 21 21)" />
        <text x={21} y={21} dominantBaseline="central" textAnchor="middle">{percentage === null ? t("--") : `${Math.round(percentage)}%`}</text>
      </svg>
      <span className="ring-label">{label}</span>
    </div>
  );
}
