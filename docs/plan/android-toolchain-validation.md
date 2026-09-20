# Android toolchain validation lane

Date: 2026-09-19

## Disposition

The disposable Android toolchain and capability probe are runnable. The API-35
emulator booted and the no-write probe produced a usable capability report.
The Android foundation build reached Kotlin compilation but is currently
blocked by an existing source error in
`android/app/src/main/kotlin/org/anycal/android/account/AnyCalAuthenticatorService.kt:52`:
`KEY_BOOLEAN_RESULT` is unresolved. This lane did not modify product code to
repair that error.

## Isolated environment

- Microsandbox VM: `any-cal-android-live-20260919`; no host mounts or personal
  profiles.
- JDK: OpenJDK 17.0.20, including `javac` after installing the full
  `openjdk-17-jdk` package.
- Gradle: 8.10.2, downloaded into the VM at `/opt/gradle`.
- Android SDK: command-line tools 22.0; platform-tools 37.0.1; emulator
  37.1.11; platform and Google APIs system image API 35.
- AVD: x86_64 Google APIs, Android 15/API 35; dedicated ADB server port 5041
  and emulator serial `emulator-5556`.
- Missing emulator runtime dependency was isolated to the VM and installed as
  `libxkbfile1`.

## Commands and evidence

Toolchain verification:

```text
java -version                 -> openjdk 17.0.20
javac -version                -> javac 17.0.20
gradle --version              -> Gradle 8.10.2
emulator -version             -> 37.1.11.0
adb version                   -> 37.0.1
```

Capability probe:

```sh
python3 scripts/android-probe/probe.py \
  --adb /opt/android-sdk/platform-tools/adb \
  --serial emulator-5556
```

Redacted result: Android 15/API 35, manufacturer `Google`, model
`sdk_gphone64_x86_64`, boot completed; `com.android.contacts` resolved to
`com.android.providers.contacts`; `com.android.calendar` resolved to
`com.android.providers.calendar`; `org.tasks.api` was absent; `org.tasks` and
the target Any-Cal package were not installed. Permission checks were
`unknown` because the target package was not installed. No account names or
provider rows were read.

Foundation build:

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon assembleDebug
```

The build installed SDK Build-Tools 34 in the disposable VM, then failed only
at Kotlin compilation with the `KEY_BOOLEAN_RESULT` error above. Gradle also
printed a non-fatal SDK XML version warning (the command-line tools understand
SDK XML through version 3 while an installed metadata file is version 4).

## Cleanup and integration notes

The VM was stopped after evidence capture. No APKs, Anytype credentials,
personal Android data, or live Anytype writes were used. The next smallest
action is to resolve the authenticator constant in the Android foundation,
then rerun the same Gradle command in this disposable toolchain. The probe
itself is independent of that compile failure and passed both offline fake
device validation and the live API-35 read-only run.

## Repeat validation after authenticator fix

Date: 2026-09-19

The source copy was refreshed in the disposable VM after the authenticator
change to `AccountManager.KEY_BOOLEAN_RESULT` landed. The exact build command
was:

```sh
ANDROID_HOME=/opt/android-sdk ANDROID_SDK_ROOT=/opt/android-sdk \
  /opt/gradle/gradle-8.10.2/bin/gradle --no-daemon :app:assembleDebug
```

Result: `BUILD SUCCESSFUL` (33 actionable tasks). The only output of note was
the pre-existing non-fatal SDK XML version 4 warning from the command-line
tools.

The API-35 emulator was booted with a dedicated ADB server on port 5041. The
probe was run with:

```sh
ANDROID_ADB_SERVER_PORT=5041 python3 /opt/probe.py \
  --adb /opt/android-sdk/platform-tools/adb --serial emulator-5556
```

Redacted live result: Android 15/API 35, `boot_completed=1`; Contacts and
Calendar authorities resolved to `com.android.providers.contacts` and
`com.android.providers.calendar`; `org.tasks.api` and `org.tasks` were absent;
the Any-Cal target package was not installed. Permission states were unknown
because no target APK was installed. The probe performed no account, contact,
calendar, task, or Anytype data reads.

The VM was stopped after evidence capture.
