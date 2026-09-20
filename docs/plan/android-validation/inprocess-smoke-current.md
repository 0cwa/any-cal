# Android in-process JNI smoke

Date: 2026-09-20

## Result

**Passed in the installed app process.** The current debug APK was rebuilt in
a fresh disposable API-35 x86_64 runtime and the debug-only
`NativeInprocessSmokeActivity` executed the native calls in-process. No
Anytype endpoint, account, credential, Space, object, provider row, DAV
client, or live write was used.

## Resource and isolation receipt

- Microsandbox: `any-cal-android-inprocess-20260920`
- Image: Ubuntu 24.04
- VM CPUs: 8 allocated / 8 maximum
- VM memory: 8,192 MiB allocated / 8,192 MiB maximum
- VM root: managed 64 GiB ext4 root disk (sparse managed allocation)
- Android: API 35 Google APIs x86_64
- Emulator: 37.1.11.0 (build 15917651)
- Emulator flags: `-no-window -no-audio -no-boot-anim -gpu
  swiftshader_indirect -accel on -no-snapshot -wipe-data -memory 2048`
- Source mount: repository mounted read-only at `/src`; build copied to
  `/work/android`; no personal Flatpak/profile or credential directory was
  mounted.

The acceleration preflight reported:

```text
KVM (version 12) is installed and usable.
```

The emulator log independently recorded `CPU Acceleration status: KVM`,
`-enable-kvm`, and no TCG fallback. The QEMU command used `-m 2560` after the
emulator's hardware-profile adjustment; the enclosing disposable VM had the
required 8 GiB allocation.

## Build and install

The current source was copied into the VM and built with Gradle 8.10.2. The
build completed successfully:

```text
BUILD SUCCESSFUL in 2m 28s
35 actionable tasks: 35 executed
lib/x86_64/libany_cal_android_bridge.so
BUILD_DONE
```

The APK installed successfully in the emulator:

```text
Performing Streamed Install
Success
package=package:/data/app/~~EKj1ZF1bldJjyRyn5x7KoQ==/org.anycal.android-5a6Vkfe7o9HzoSwGolbLqg==/base.apk
```

The resolved activity was:

```text
org.anycal.android/.NativeInprocessSmokeActivity
```

## In-process assertions

The activity was launched with `am start -W` and emitted these redacted
logcat lines from the installed app UID:

```text
09-20 09:20:38.921 2040 2135 I AnyCalNativeSmoke: stage=readiness available=true negotiate=true health=true
09-20 09:20:38.926 2040 2135 I AnyCalNativeSmoke: stage=valid_response length=124 not_linked=true
09-20 09:20:38.926 2040 2135 I AnyCalNativeSmoke: stage=malformed error=IllegalArgumentException redacted=true
09-20 09:20:38.927 2040 2135 I AnyCalNativeSmoke: available=true negotiate=true health=true valid_not_linked=true malformed=redacted
```

The bounded command reported:

```text
boot_completed=1
apk_install_rc=0
SMOKE_RESULT=PASS
post_activity_state=device
```

This proves, for the installed x86_64 debug package:

1. `NativeRustBridge.available()` returned `true`.
2. `NativeRustBridge.negotiate()` returned `true`.
3. `NativeRustBridge.health()` returned `true`.
4. A valid credential-free request returned a response containing
   case-insensitive `NOT_LINKED`.
5. A malformed request returned `IllegalArgumentException`, with only the
   exception class and `redacted=true` logged.

## Cleanup and independent verification

The activity command first shut down the emulator, then the guest ADB server
was stopped. The guest process check returned no matching emulator, QEMU, or
ADB process. The disposable runtime was then removed:

```text
msb stop any-cal-android-inprocess-20260920 -> Stopped
msb remove any-cal-android-inprocess-20260920 -> Removed
msb status -a | grep any-cal-android-inprocess-20260920 -> no output
```

An independent host process-name check found no emulator or QEMU process. A
host ADB server with PID `331715` was already present before this run and
remained afterward. Stopping that host-wide daemon was rejected as unsafe
because it may belong to an unrelated workflow; the smoke run used the guest
ADB server and did not create the host daemon. Therefore the disposable VM
and all processes created by this run are clean, while the stronger
host-wide “no ADB process” condition remains separately unverified.

## Disposition

The in-process JNI assertions and KVM runtime gate are **passed**. The
host-wide ADB absence condition is **not claimed** because of the pre-existing
daemon. This receipt does not claim Anytype transport, provider CRUD, account
lifecycle, Tasks.org, DAVx5, bidirectional sync, or live network behavior.
