export function SearchBox({ value, onChange }: { value: string; onChange: (value: string) => void }) {
  return (
    <input
      type="search"
      className="input max-w-xs"
      placeholder="Search notes…"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  );
}
