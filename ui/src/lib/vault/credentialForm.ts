// Draft/validation for the credential editor. Pure; secrets live only in the draft while the
// dialog is open and are cleared by the component on save, cancel and destroy.

import type { CredentialInput, CredentialKind, CredentialSummary, Uuid } from "../ipc";
import type { FieldErrors } from "../editors/form";

export interface CredentialFormDraft {
  id: Uuid | null;
  kind: CredentialKind;
  name: string;
  username: string;
  domain: string;
  password: string;
  privateKey: string;
  passphrase: string;
}

export type CredentialField = "name" | "password" | "privateKey";

export function newCredentialDraft(kind: CredentialKind = "password"): CredentialFormDraft {
  return {
    id: null,
    kind,
    name: "",
    username: "",
    domain: "",
    password: "",
    privateKey: "",
    passphrase: "",
  };
}

/** Editing starts without secrets: an empty secret means "keep the current one". */
export function draftFromSummary(s: CredentialSummary): CredentialFormDraft {
  return {
    ...newCredentialDraft(s.kind),
    id: s.id,
    name: s.name,
    username: s.username,
    domain: s.domain ?? "",
  };
}

export function secretIsEmpty(d: CredentialFormDraft): boolean {
  return d.kind === "password" ? d.password === "" : d.privateKey.trim() === "";
}

export function validateCredentialDraft(d: CredentialFormDraft): FieldErrors<CredentialField> {
  const errors: FieldErrors<CredentialField> = {};
  if (d.name.trim() === "") errors.name = "cred.required.name";
  // When editing, an empty secret keeps the stored one.
  if (d.id === null && secretIsEmpty(d)) {
    errors[d.kind === "password" ? "password" : "privateKey"] =
      d.kind === "password" ? "cred.required.password" : "cred.required.key";
  }
  return errors;
}

/** Metadata part, shared by "new/replace secret" and "keep secret" saves. */
export function draftMetadata(d: CredentialFormDraft): Omit<CredentialInput, "secret" | "id"> {
  const domain = d.domain.trim();
  return { name: d.name.trim(), username: d.username.trim(), domain: domain === "" ? null : domain };
}

export function draftToInput(d: CredentialFormDraft): CredentialInput {
  const secret: CredentialInput["secret"] =
    d.kind === "password"
      ? { type: "password", password: d.password }
      : {
          type: "sshKey",
          privateKeyPem: d.privateKey,
          passphrase: d.passphrase === "" ? null : d.passphrase,
        };
  return { id: d.id, ...draftMetadata(d), secret };
}
