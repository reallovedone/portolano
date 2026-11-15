// The credential-prompt flow of the mock IPC, shared by the fake terminal and the fake
// framebuffer so both speak the same protocol as the real backend (WP11):
//
//   open -> [host key / certificate prompts] -> credentialPrompt -> answer ->
//     accepted: the session starts;  rejected: the prompt comes back with an error;
//     cancelled: `error` with the `cancelled` code.
//
// NOTHING here is secure. The mock "server" accepts any non-empty secret except the password
// `MOCK_REJECTED_PASSWORD`, so the error path can be exercised by hand.

import { IpcError } from "./errors";
import type {
  Connection,
  CredentialAnswer,
  CredentialPrompt,
  CredentialPromptKind,
} from "./types";

/** The one password the mock server always rejects. */
export const MOCK_REJECTED_PASSWORD = "wrong";
/** Failed attempts after which the mock gives up (the real backend caps them too). */
export const MOCK_MAX_ATTEMPTS = 5;

/** What the owner of the mock (`createMockIpc`) knows and the sessions do not. */
export interface MockPromptHooks {
  /**
   * How a session for `connection` gets its credential: from the vault (no prompt) or by
   * asking. Throws `vault_locked` like the real backend when the vault must be unlocked.
   */
  decide(connection: Connection, askRequested: boolean): "vault" | "ask";
  /** Whether "save to vault" is offered (a vault exists and is unlocked). */
  canSave(): boolean;
  /** Called with a credential the user chose to keep, once authentication succeeded. */
  saved(connection: Connection, answer: CredentialAnswer): void;
}

/** No prompts at all: every session uses its (imaginary) vault credential. */
export const NO_PROMPTS: MockPromptHooks = {
  decide: () => "vault",
  canSave: () => false,
  saved: () => undefined,
};

function target(connection: Connection): string {
  return `${connection.host}:${connection.port ?? ""}`;
}

export interface PrompterCallbacks {
  emit(prompt: CredentialPrompt): void;
  /** The credentials were accepted: start the session. */
  granted(): void;
  /** The user cancelled, or the attempts ran out. */
  failed(code: "cancelled" | "auth_failed", message: string): void;
}

/** One session's pending credential prompt. */
export class MockPrompter {
  #waiting = false;
  #failures = 0;
  #lastUsername: string | null;
  readonly #connection: Connection;
  readonly #kind: CredentialPromptKind;
  readonly #hooks: MockPromptHooks;
  readonly #callbacks: PrompterCallbacks;

  constructor(
    connection: Connection,
    kind: CredentialPromptKind,
    hooks: MockPromptHooks,
    callbacks: PrompterCallbacks,
  ) {
    this.#connection = connection;
    this.#kind = kind;
    this.#hooks = hooks;
    this.#callbacks = callbacks;
    this.#lastUsername = connection.username;
  }

  get waiting(): boolean {
    return this.#waiting;
  }

  ask(error: string | null = null): void {
    this.#waiting = true;
    const domain =
      this.#connection.options.type === "rdp" ? this.#connection.options.domain : null;
    this.#callbacks.emit({
      kind: this.#kind,
      usernameHint: this.#lastUsername,
      domainHint: domain,
      target: target(this.#connection),
      error,
      canSaveToVault: this.#hooks.canSave(),
    });
  }

  #claim(): void {
    if (!this.#waiting) throw new IpcError("invalid_input", "no credential prompt is pending");
    this.#waiting = false;
  }

  answer(answer: CredentialAnswer, saveToVault: boolean): void {
    this.#claim();
    this.#lastUsername = answer.username.trim() === "" ? this.#lastUsername : answer.username.trim();
    let problem: string | null = null;
    if (this.#kind !== "password" && answer.username.trim() === "") problem = "Enter a user name.";
    else if ((answer.password ?? "") === "" && (answer.keyPath ?? "") === "") {
      problem = this.#kind === "sshKey" ? "Enter a password or choose a private key file." : "Enter a password.";
    } else if (answer.password === MOCK_REJECTED_PASSWORD) {
      problem = "The server rejected the credentials.";
    }
    if (problem !== null) {
      this.#failures += 1;
      if (this.#failures >= MOCK_MAX_ATTEMPTS) {
        this.#callbacks.failed("auth_failed", `Too many failed attempts to authenticate to ${target(this.#connection)}.`);
        return;
      }
      this.ask(problem);
      return;
    }
    if (saveToVault && this.#hooks.canSave()) this.#hooks.saved(this.#connection, answer);
    this.#callbacks.granted();
  }

  cancel(): void {
    this.#claim();
    this.#callbacks.failed("cancelled", "No credential was provided.");
  }
}
