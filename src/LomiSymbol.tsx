// Geometry from brandbook/logo/svg/lomi-symbol-*.svg (708 x 780 grid).
const M =
  "M416 468C416 428.2 448.2 396 488 396C527.8 396 560 428.2 560 468V780H696V467H696C695.5 352.6 602.5 260 488 260C434.1 260 385 280.5 348 314.2C311 280.5 261.9 260 208 260C93.5 260 0.5 352.6 0 467H0V780H136V468C136 428.2 168.2 396 208 396C247.8 396 280 428.2 280 468V780H416Z";

export default function LomiSymbol({
  size = 32,
  className,
}: {
  size?: number;
  className?: string;
}) {
  return (
    <svg
      className={className ? `lomi-symbol ${className}` : "lomi-symbol"}
      viewBox="0 0 708 780"
      width={size}
      height={(size * 780) / 708}
      aria-hidden="true"
    >
      <path className="lomi-symbol-m" d={M} fill="currentColor" />
      <rect
        className="lomi-symbol-signal"
        x="548"
        y="0"
        width="160"
        height="160"
        rx="52"
      />
    </svg>
  );
}
