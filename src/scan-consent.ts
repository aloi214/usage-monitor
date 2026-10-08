// Local source grants and remote account grants are deliberately separate.
export function scanSpendVisible(
  row: { id: string; sources?: string[] },
  roots: Record<string, string[]>,
  cursorAuthorized: boolean,
): boolean {
  if (row.sources?.length) return row.sources.every((source) => Boolean(roots[source]?.length));
  return row.id === "cursor" && cursorAuthorized;
}

export function acceptSpendBatch(requestEpoch: number, currentEpoch: number, generation: number, lastApplied: number): boolean {
  return requestEpoch === currentEpoch && generation >= lastApplied;
}
