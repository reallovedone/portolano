// Shared helpers for the field components (TextField, TextArea, Select).

/** Ids of the hint and error elements for a control id. */
export function fieldIds(id: string): { hint: string; error: string } {
  return { hint: `${id}-hint`, error: `${id}-err` };
}

/** Value for aria-describedby: only the elements that are actually rendered. */
export function describedBy(id: string, hint?: string, error?: string | null): string | undefined {
  const ids = fieldIds(id);
  const parts = [hint ? ids.hint : "", error ? ids.error : ""].filter(Boolean);
  return parts.length > 0 ? parts.join(" ") : undefined;
}
