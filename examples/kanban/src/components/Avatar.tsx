// Deterministic per-user avatar: no external asset, no illustration
// library — just a hash of the user id driving a shape choice and a hue,
// rendered as inline SVG. Same id always produces the same avatar, so a
// peer is visually recognizable across reconnects without the server
// ever having to store or serve anything about them.
const SHAPES = ["circle", "hexagon", "square"] as const;
type Shape = (typeof SHAPES)[number];

function hashId(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) {
    h = (h << 5) - h + id.charCodeAt(i);
    h |= 0;
  }
  return Math.abs(h);
}

function shapePath(shape: Shape): JSX.Element {
  if (shape === "circle") return <circle cx="18" cy="18" r="17" />;
  if (shape === "hexagon") return <polygon points="18,1.5 33,9.5 33,26.5 18,34.5 3,26.5 3,9.5" />;
  return <rect x="2" y="2" width="32" height="32" rx="9" />;
}

export function Avatar({ id, size = 28 }: { id: string; size?: number }) {
  const h = hashId(id);
  const hue = h % 360;
  const shape = SHAPES[h % SHAPES.length];
  const eyeDx = shape === "hexagon" ? 6 : 5.5;

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 36 36"
      className="avatar"
      style={{ color: `hsl(${hue} 70% 55%)` }}
      aria-hidden="true"
    >
      <g fill="currentColor">{shapePath(shape)}</g>
      <circle cx={18 - eyeDx} cy="19" r="2.4" fill="#0b0e14" />
      <circle cx={18 + eyeDx} cy="19" r="2.4" fill="#0b0e14" />
    </svg>
  );
}
