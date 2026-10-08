// Price publication has its own epoch, independent of query refresh ordering.
export function acceptPriceEpoch(request: number, current: number): boolean { return request === current; }
export function mergeRepricedLocal<T extends { sources: string[]; scan_revision?: number | null }>(
  current: T[], updated: T[], revision: number, visible: (row: T) => boolean,
): T[] {
  return [
    ...current.filter((row) => row.sources.length === 0 && visible(row)),
    ...updated.filter((row) => row.sources.length > 0 && row.scan_revision === revision && visible(row)),
  ];
}
export function estimatedCostText(
  formatted: string, cost: number, unpriced: number,
  translate: (key: string, vars?: Record<string, string | number>) => string,
): string {
  return unpriced > 0
    ? (cost > 0 ? translate("pricing.costPlusUnknown", { cost: formatted }) : translate("pricing.unknown"))
    : formatted;
}
