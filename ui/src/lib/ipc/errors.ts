import type { AppError, ErrorCode } from "./types";

const ERROR_CODES: readonly ErrorCode[] = [
  "vault_locked",
  "auth_failed",
  "host_unreachable",
  "host_key_mismatch",
  "not_found",
  "invalid_input",
  "io",
  "unsupported",
  "internal",
  "cancelled",
  "security_downgrade",
  "quick_unlock_cancelled",
  "quick_unlock_failed",
  "quick_unlock_not_ready",
  "read_only",
  "conflict",
];

/** The error every IPC wrapper throws: carries the stable backend `code`. */
export class IpcError extends Error {
  readonly code: ErrorCode;

  constructor(code: ErrorCode, message: string) {
    super(message);
    this.name = "IpcError";
    this.code = code;
  }
}

function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.code === "string" &&
    (ERROR_CODES as readonly string[]).includes(v.code) &&
    typeof v.message === "string"
  );
}

/** Turns whatever `invoke` rejected with into an `IpcError`. */
export function toIpcError(e: unknown): IpcError {
  if (e instanceof IpcError) return e;
  if (isAppError(e)) return new IpcError(e.code, e.message);
  if (e instanceof Error) return new IpcError("internal", e.message);
  return new IpcError("internal", typeof e === "string" ? e : "unknown IPC failure");
}
