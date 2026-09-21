## Summary

Describe the user/developer-visible outcome and the boundary of the change.

## Validation

List the exact commands, fixtures, or CI jobs used. Call out anything intentionally not
validated (for example live Anytype, a real DAV client, or an Android device).

## Risk and handoff

Describe compatibility or migration risk, security/recovery impact, and any follow-up
issue that should be completed separately.

## Checklist

- [ ] The change keeps Anytype as the authoritative durable store.
- [ ] Protocol/device data that cannot be projected losslessly is preserved.
- [ ] Newly advertised behavior is covered by deterministic tests or fixtures.
- [ ] No credentials, personal data, or unsanitized external artifacts were added.
- [ ] Relevant README/architecture documentation was updated.
- [ ] Deferred work is linked to a prioritized GitHub issue instead of a bare TODO.
