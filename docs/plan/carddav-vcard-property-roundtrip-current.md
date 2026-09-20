# Local CardDAV vCard property round-trip receipt

Date: 2026-09-20  
Scope: offline synthetic vCards and the local `any-cal-core` parser,
serializer, and Contact projection only.

## Result

The bounded property-round-trip lane passes. The focused fixture covers:

- structured `N` and `ADR` values;
- repeated `EMAIL` and `TEL` values with per-occurrence parameters;
- grouped `ITEM1.EMAIL` properties;
- quoted parameters containing commas and semicolons;
- escaped commas, semicolons, and newlines;
- Unicode content;
- an unknown `X-*` property and a parameter containing a literal backslash;
- partial projection updates that change `FN` while preserving all other and
  unknown occurrences.

## Reproduced defect and fix

The synthetic fixture initially found that vCard value unescaping was also
applied to parameter values. That changed a literal backslash in an `X-*`
parameter during parse/serialize/parse. Parameter values are now retained from
the parameter-head parser, which already handles quoted-string decoding;
vCard value escaping remains limited to property values.

No broad RFC or external-client behavior was inferred from this fix.

## Verification

All commands were run with `LD_PRELOAD` explicitly unset:

```text
env -u LD_PRELOAD /home/x/.cargo/bin/cargo test -p any-cal-core --tests
  4 unit + 5 contract + 4 iCalendar + 6 repository + 7 round-trip tests passed
env -u LD_PRELOAD /home/x/.cargo/bin/cargo clippy -p any-cal-core --all-targets -- -D warnings
  passed
env -u LD_PRELOAD /home/x/.cargo/bin/cargo fmt --all -- --check
  passed
```

No network, external client, Anytype credential/state, Android/provider,
personal Flatpak, or pending credential artifact was accessed.

## Boundary

This receipt proves local semantic round trips and projection preservation. It
does not claim interoperability with DAVx5, Tasks.org, or any other external
CardDAV client.
