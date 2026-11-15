// Rough master-password strength hint. Purely advisory: the backend decides what is accepted.

export const MIN_MASTER_PASSWORD_LENGTH = 8;

export type Strength = "empty" | "weak" | "fair" | "good" | "strong";

export function passwordStrength(password: string): Strength {
  if (password.length === 0) return "empty";
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((re) => re.test(password)).length;
  const distinct = new Set(password).size;
  let score = 0;
  if (password.length >= 8) score += 1;
  if (password.length >= 12) score += 1;
  if (password.length >= 16) score += 1;
  if (classes >= 3) score += 1;
  if (distinct < Math.min(6, password.length)) score = Math.max(0, score - 2);
  if (password.length < 8) return "weak";
  if (score <= 1) return "weak";
  if (score === 2) return "fair";
  if (score === 3) return "good";
  return "strong";
}
