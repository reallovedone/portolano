# Vendored crates

## `vendor/sspi` (sspi 0.23.0 plus one backported fix)

`vendor/sspi/` is the published `sspi` 0.23.0 crate, unchanged except for a backport of the
upstream fix for the Kerberos `mechListMIC` sequence number. It is wired in through
`[patch.crates-io]` in the root `Cargo.toml`, and excluded from the workspace (`exclude`), so it
is not formatted, linted or tested with our crates (CI runs its Kerberos tests separately). `Cargo.lock` still reports version 0.23.0.

### Why

With Kerberos (CredSSP over SPNEGO) against a real Windows Server, sspi 0.23.0 fails with
`InvalidToken: invalid MIC token sequence number: expected <AP-REP seq> but got 0` when it
verifies the server's SPNEGO `mechListMIC`: Windows signs that MIC with sequence number 0, not
with the AP-REP sequence number. This was fixed upstream in sspi 0.23.1
([issue #748](https://github.com/Devolutions/sspi-rs/issues/748),
[PR #772](https://github.com/Devolutions/sspi-rs/pull/772), merge commit `08d8622`, plus the
follow-up `c65cc2f` "fix(pku2u): enforce MIC sequence numbers").

We cannot simply upgrade: `ironrdp-connector` 0.12.0 (the latest release) pins
`picky = "=7.0.0-rc.26"`, while sspi 0.23.1 pins `=7.0.0-rc.27`, so the two cannot be resolved
together. Until IronRDP moves to a compatible sspi, we vendor 0.23.0 with only the fix.

### What was changed

Only the source changes of PR #772 (in its final state after `c65cc2f`) were applied; the
`picky-krb` 0.12 to 0.13 bump and the other manifest and lockfile changes of the PR were
deliberately NOT applied (that bump is what causes the incompatibility). Files changed in the
vendored copy:

- `src/kerberos/utils.rs`: `validate_mic_token` takes a new `enforce_seq_number` flag that gates
  the sequence-number comparison.
- `src/kerberos/mod.rs`: `Kerberos::verify_mic_token` enforces the sequence number only on the
  acceptor side (`!self.is_client()`); the initiator no longer does.
- `src/pku2u/mod.rs`: `verify_mic` forwards the flag; both PKU2U call sites pass `true`, so
  PKU2U keeps enforcing sequence numbers. Test setup moved into a `minimal_pku2u_pair` helper
  and two tests added.
- `src/kerberos/tests.rs`: two tests added (initiator accepts a MIC with sequence number 0,
  acceptor rejects it).

Not compiled here, so removed: the upstream `.github/`, `tools/`, `examples/` and the
integration tests in `tests/`, with their `[[example]]` and `[[test]]` entries in `Cargo.toml`.
`Cargo.toml.orig`, licences and every other file are as published. CI runs the library's Kerberos
and PKU2U unit tests (`cargo test --manifest-path vendor/sspi/Cargo.toml --locked --lib --
kerberos pku2u`). To review the
exact change: `diff -ru ~/.cargo/registry/src/*/sspi-0.23.0 vendor/sspi` (with the 0.23.0
crate downloaded), or compare against the upstream PR.

### Security impact

The MIC checksum is still fully verified with the acceptor subkey, so the token stays
authenticated and cannot be forged or altered. Only the check that the acceptor's MIC carries a
specific sequence number is dropped on the Kerberos initiator side (the side Portolano uses),
exactly as upstream does: the client does not negotiate sequence or replay detection, and
Windows always uses 0 here. The acceptor side and PKU2U keep enforcing it.

### How and when to remove it

When an `ironrdp-connector` release depends on `sspi >= 0.23.1` (check that its `picky` pin
matches), remove the patch:

1. Delete `vendor/sspi/`.
2. Delete the `[patch.crates-io]` section and the `exclude = ["vendor/sspi"]` line from the root
   `Cargo.toml`.
3. Delete this file and the reference to it in `README.md`.
4. Run `cargo update -p ironrdp-connector -p sspi` and the full check suite.

## Kerberos behaviour to know about

- When the domain controller (KDC) cannot be reached, or does not answer within 5 seconds,
  Portolano makes one more connection with NTLM only, which is what 0.3.0 did. Blocking port 88
  (or a firewall dropping it) therefore downgrades the sign-in to NTLM instead of failing. If
  the NTLM attempt fails for a reason of its own (wrong password, server without NLA, changed
  certificate) that reason is shown; only a network failure is reported as "the domain
  controller could not be reached".
- A connection imported from a file never carries a Kerberos KDC: the importer removes it and
  warns, because the KDC receives the sign-in request of the domain account.
- The logs of `sspi` are switched off completely, because even its error-level events can quote
  user and realm names (the Kerberos AS exchange error is logged with `?err`).
