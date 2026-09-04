/** Mirrors `--spacing-row` / `--spacing-row-compact` in `index.css`. The
 * row virtualizer measures in numbers, so the two heights live here as
 * well — changing one means changing the other. */
export const ROW_HEIGHT = { comfortable: 32, compact: 26 } as const;

export type Density = keyof typeof ROW_HEIGHT;
