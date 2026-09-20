# Anytype CLI app-list parser correction

Updated 2026-09-20. This bounded lane used only the redacted structural
capture in `anytype-cli-app-list-output-capture-current.md`; it did not invoke
the CLI or access credentials, network, providers, objects, Spaces, or
schemas.

## Changes

- The row fixture now models the observed token-length sequence
  `7,10,3,1,64,11,10,8` using synthetic values only.
- App IDs are accepted only as exactly 64 ASCII hexadecimal characters and
  are returned only as a SHA-256 fingerprint.
- ANSI escape bytes and other non-layout control characters are rejected
  before normalization. Spaces, tabs, and normal line endings remain valid
  layout whitespace.
- Offline tests cover the observed shape, flexible whitespace, ANSI/control
  contamination, wrong ID shape, extra rows, ambiguity, empty output,
  malformed timestamps, unredacted keys, private-file permissions, and
  secret redaction.

## Validation

```text
cargo fmt --check                         PASS
LD_PRELOAD= RUSTC_WRAPPER= cargo test --manifest-path tools/anytype-probe/Cargo.toml
                                           PASS: 24 passed, 0 failed
```

The first test invocation inherited the host's hardened allocator preload and
aborted while running `rustc -vV`; the same offline command with those two
environment variables explicitly empty completed successfully. No live or
external operation was attempted.

The parser is ready for a separately authorized fresh lifecycle gate. This
lane does not establish app-link handoff, revocation, service restart, or
post-revocation authentication.
