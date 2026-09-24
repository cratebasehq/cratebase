// Deterministically hashes an id into a color + initial — same id always
// renders the same avatar, no external image service or per-user data
// beyond what's already in `users`/`presence`.
function hash(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) | 0;
  return Math.abs(h);
}

const HUES = [12, 28, 145, 205, 265, 320];

export function Avatar({ id, name, size = 24 }: { id: string; name: string; size?: number }) {
  const hue = HUES[hash(id) % HUES.length];
  const initial = (name.trim()[0] || "?").toUpperCase();
  return (
    <span
      className="inline-flex shrink-0 items-center justify-center rounded-full font-display font-semibold text-white"
      style={{ width: size, height: size, fontSize: size * 0.42, backgroundColor: `hsl(${hue} 55% 42%)` }}
      title={name}
    >
      {initial}
    </span>
  );
}
