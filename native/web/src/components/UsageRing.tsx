import { t } from "../i18n";

const RADIUS = 25;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** Usage ring: thin arc below 70 %, thick from 70 %, thick with a "!" badge from 90 %; "--" while unknown. */
export function UsageRing({ value, label }: { value: number | null; label: string }) {
  const percentage = value === null || !Number.isFinite(value) ? null : Math.max(0, Math.min(value, 100));
  const level = percentage === null ? "unknown" : percentage >= 90 ? "crit" : percentage >= 70 ? "warn" : "ok";
  const filled = percentage === null ? 0 : (percentage / 100) * CIRCUMFERENCE;
  const strokeWidth = level === "warn" || level === "crit" ? 6 : 4.5;
  return (
    <div className="ring" data-level={level} role="img" aria-label={percentage === null ? t("{label}: not available", { label }) : t("{label}: {value}%", { label, value: Math.round(percentage) })}>
      <svg width={48} height={48} viewBox="0 0 60 60" aria-hidden="true">
        <circle className="ring-track" cx={30} cy={30} r={RADIUS} fill="none" strokeWidth={strokeWidth} />
        <circle className="ring-arc usage-ring-progress" cx={30} cy={30} r={RADIUS} fill="none" strokeWidth={strokeWidth} strokeLinecap="round"
          strokeDasharray={`${filled} ${CIRCUMFERENCE}`} opacity={filled > 0 ? 1 : 0} transform="rotate(-90 30 30)" />
        <text x={30} y={30.5} dominantBaseline="central" textAnchor="middle">{percentage === null ? t("--") : `${Math.round(percentage)}%`}</text>
        {level === "crit" && <g className="ring-badge"><circle cx={52} cy={8} r={8} /><path d="M52 4.5v4M52 11.5h.01" /></g>}
      </svg>
      <span className="ring-label">{label}</span>
    </div>
  );
}
