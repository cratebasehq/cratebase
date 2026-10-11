export function SearchBox({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return (
    <input
      type="search"
      className="input"
      aria-label="Search notes"
      placeholder="Search notes…"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
