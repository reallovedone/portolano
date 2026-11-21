const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "0.3.1 — 18 Nov 2026"; just the version while it has no release date. */
export function versionHeading(version: string, date: string | null): string {
  const match = date === null ? null : /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const month = match ? MONTHS[Number(match[2]) - 1] : undefined;
  if (!match || month === undefined) return version;
  return `${version} — ${Number(match[3])} ${month} ${match[1]}`;
}
