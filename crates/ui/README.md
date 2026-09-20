# any-cal-gui

This is a thin Slint configuration/status client for the headless app. It
shares `AppConfig`; it does not own Anytype data, DAV state, or durability.
Build with the cached Slint 1.12.1 toolchain. The UI masks token state and
never displays or logs the token. The headless service defaults to live HTTP;
the GUI only configures it and queries its local health endpoint. Deterministic
fake transport is an explicit service test override. Live Anytype endpoint and
account compatibility still require deployment-specific validation. The
markup is display-independent at compile/test time;
runtime requires a native Slint backend and a desktop display. Labels and
status roles are provided for keyboard and assistive-technology basics.

The GUI currently has no OS secret-store dependency. It can read legacy
`token=` entries for migration, but refuses to save any configuration while a
token is present, so it never writes a new plaintext token. Supply the token
through `ANY_CAL_ANYTYPE_TOKEN` or the CLI until platform secret-store support
is added. For a proxy-protected local service, set `ANY_CAL_LOCAL_AUTH` in the
GUI process environment or enter the runtime-only local health credential in
the UI. It is sent only to `/health` and is omitted from saved configuration.
