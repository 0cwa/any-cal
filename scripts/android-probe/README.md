# Android provider capability probe

`probe.py` is a no-write, redacted capability probe for an isolated Android
emulator/device. It distinguishes an unavailable ADB environment from an
available device where Contacts, Calendar, or the optional Tasks.org provider
is absent. It does not enumerate account names or contact/calendar/task rows.

## Run

```sh
python3 scripts/android-probe/probe.py --serial 127.0.0.1:5557 \
  > capability.json
```

If exactly one device is attached, `--serial` may be omitted. Use a dedicated
ADB server for an isolated emulator:

```sh
ANDROID_ADB_SERVER_PORT=5041 adb -P 5041 start-server
ANDROID_ADB_SERVER_PORT=5041 python3 scripts/android-probe/probe.py \
  --adb "$ANDROID_SDK_ROOT/platform-tools/adb" --serial 127.0.0.1:5557
```

Exit status is `0` for a device report and `2` for an unavailable environment
or an invalid requested serial. The JSON report includes API/build metadata,
provider authority resolution and owner package, Tasks.org package/version,
target-package installation state, bounded permission checks, and an explicit
account/provider ownership boundary. It intentionally reports no account names
and no provider row payloads.

## Offline validation

```sh
python3 -m py_compile scripts/android-probe/probe.py
python3 scripts/android-probe/probe.py --help
```

Live acceptance requires a disposable API-35+ emulator and `adb`; no host
SDK, emulator, APK, Anytype credential, or personal Android profile is
implicitly created by this harness.
