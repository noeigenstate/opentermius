const RADIUS = 17;
const CIRCUMFERENCE = 2 * Math.PI * RADIUS;

/** Usage ring: green below 70 %, amber below 90 %, red above; "—" while unknown. */
export function UsageRing({ value, label }: { value: number | null; label: string }) {
  const tone = value === null ? "var(--muted)" : value >= 90 ? "var(--danger)" : value >= 70 ? "var(--warning)" : "var(--success)";
  const filled = value === null ? 0 : (Math.min(value, 100) / 100) * CIRCUMFERENCE;
  return (
    <div className="ring">
      <svg width={42} height={42} viewBox="0 0 42 42">
        <circle cx={21} cy={21} r={RADIUS} fill="none" stroke="var(--ring-track)" strokeWidth={3} />
        {filled > 0 && (
          <circle cx={21} cy={21} r={RADIUS} fill="none" stroke={tone} strokeWidth={3} strokeLinecap="round"
            strokeDasharray={`${filled} ${CIRCUMFERENCE}`} transform="rotate(-90 21 21)" />
        )}
        <text x={21} y={21} dominantBaseline="central" textAnchor="middle">{value === null ? "—" : `${value}%`}</text>
      </svg>
      <span className="ring-label">{label}</span>
    </div>
  );
}
