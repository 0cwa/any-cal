#!/usr/bin/env python3
"""No-write Android provider capability probe.

The probe deliberately reports capability metadata only.  It never inserts,
updates, deletes, or enumerates contact/calendar/task payloads.  Use --serial
when more than one disposable device is attached.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
from typing import Any


def run(adb: str, serial: str | None, *args: str, timeout: float = 8.0) -> tuple[int, str]:
    command = [adb]
    if serial:
        command += ["-s", serial]
    command += list(args)
    try:
        completed = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return 124, str(exc)
    return completed.returncode, (completed.stdout + completed.stderr).strip()


def prop(adb: str, serial: str, name: str) -> str | None:
    code, output = run(adb, serial, "shell", "getprop", name)
    return output if code == 0 and output else None


def resolve_provider(adb: str, serial: str, authority: str) -> dict[str, Any]:
    code, output = run(
        adb,
        serial,
        "shell",
        "cmd",
        "package",
        "resolve-content-provider",
        "--brief",
        authority,
    )
    value = output.splitlines()[-1].strip() if output else ""
    package = value.split("/")[0] if "/" in value else None
    resolution_status = "resolved" if code == 0 and package else "probe_error"
    # API 35's `cmd package` shell command does not expose the older
    # resolve-content-provider verb. `dumpsys package providers` is read-only
    # and remains available on API 35+.
    if not package:
        dump_code, dump = run(adb, serial, "shell", "dumpsys", "package", "providers")
        match = re.search(
            rf"\[{re.escape(authority)}\]:\s*\n\s*Provider\{{[^}}]*\s+([^/\s]+)/",
            dump,
        )
        if dump_code == 0 and match:
            package = match.group(1)
            resolution_status = "resolved"
        elif dump_code == 0:
            resolution_status = "absent"
    absent = package is None
    return {
        "authority": authority,
        "available": not absent,
        "owner_package": package,
        "resolution_status": resolution_status if not absent else resolution_status,
    }


def package_info(adb: str, serial: str, package: str) -> dict[str, Any]:
    code, path = run(adb, serial, "shell", "pm", "path", package)
    installed = code == 0 and "package:" in path
    result: dict[str, Any] = {"package": package, "installed": installed}
    if not installed:
        result["version_name"] = None
        result["version_code"] = None
        return result
    _, dump = run(adb, serial, "shell", "dumpsys", "package", package)
    name = re.search(r"versionName=([^\s]+)", dump)
    code_match = re.search(r"versionCode=([^\s]+)", dump)
    result["version_name"] = name.group(1) if name else None
    result["version_code"] = code_match.group(1) if code_match else None
    return result


def permission_status(adb: str, serial: str, package: str, permission: str) -> str:
    code, output = run(adb, serial, "shell", "cmd", "package", "check-permission", permission, package, "0")
    if code != 0:
        return "unknown"
    lowered = output.lower()
    if "granted" in lowered:
        return "granted"
    if "denied" in lowered:
        return "denied"
    return "unknown"


def choose_serial(adb: str, requested: str | None) -> tuple[str | None, list[str], str | None]:
    code, output = run(adb, None, "devices")
    if code != 0:
        return None, [], "adb_unavailable"
    devices = []
    for line in output.splitlines()[1:]:
        fields = line.split()
        if fields and len(fields) >= 2 and fields[1] == "device":
            devices.append(fields[0])
    if requested:
        if requested in devices:
            return requested, devices, None
        return None, devices, "requested_serial_unavailable"
    if len(devices) == 1:
        return devices[0], devices, None
    if not devices:
        return None, devices, "no_device"
    return None, devices, "multiple_devices_require_serial"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--adb", default=os.environ.get("ADB", shutil.which("adb") or "adb"))
    parser.add_argument("--serial", default=os.environ.get("ANDROID_SERIAL"))
    parser.add_argument("--target-package", default="com.anycal.app")
    args = parser.parse_args()

    result: dict[str, Any] = {
        "schema": "any-cal.android-capability-probe.v1",
        "mode": "read-only",
        "environment": "unknown",
        "adb": {"path": args.adb, "requested_serial": args.serial},
    }
    if not os.path.exists(args.adb) and shutil.which(args.adb) is None:
        result["environment"] = "unavailable"
        result["blocker"] = "adb_not_found"
        print(json.dumps(result, indent=2, sort_keys=True))
        return 2

    serial, devices, error = choose_serial(args.adb, args.serial)
    result["adb"]["devices"] = devices
    if error or not serial:
        result["environment"] = "unavailable"
        result["blocker"] = error
        print(json.dumps(result, indent=2, sort_keys=True))
        return 2

    result["environment"] = "available"
    result["adb"]["serial"] = serial
    result["device"] = {
        "manufacturer": prop(args.adb, serial, "ro.product.manufacturer"),
        "model": prop(args.adb, serial, "ro.product.model"),
        "android_release": prop(args.adb, serial, "ro.build.version.release"),
        "api_level": prop(args.adb, serial, "ro.build.version.sdk"),
        "boot_completed": prop(args.adb, serial, "sys.boot_completed"),
    }
    result["providers"] = {
        "contacts": resolve_provider(args.adb, serial, "com.android.contacts"),
        "calendar": resolve_provider(args.adb, serial, "com.android.calendar"),
        "tasks_org": resolve_provider(args.adb, serial, "org.tasks.api"),
    }
    tasks_package = package_info(args.adb, serial, "org.tasks")
    result["packages"] = {"tasks_org": tasks_package, "target": package_info(args.adb, serial, args.target_package)}

    # Permission checks are intentionally limited to the configured target and
    # Tasks.org's documented optional permissions; no account or row payload is read.
    permissions = [
        "android.permission.READ_CONTACTS",
        "android.permission.WRITE_CONTACTS",
        "android.permission.READ_CALENDAR",
        "android.permission.WRITE_CALENDAR",
    ]
    if tasks_package["installed"]:
        permissions += ["org.tasks.permission.READ_TASKS", "org.tasks.permission.WRITE_TASKS"]
    result["permissions"] = {
        permission: permission_status(args.adb, serial, args.target_package, permission)
        for permission in permissions
    }
    result["account_provider_boundary"] = {
        "target_package_installed": result["packages"]["target"]["installed"],
        "account_enumeration": "not_performed",
        "provider_owner_packages": {
            key: value["owner_package"] for key, value in result["providers"].items()
        },
        "note": "A provider owner is not an Any-Cal account. Account ownership requires the Android adapter package and authenticator; this no-write probe does not enumerate account names.",
    }
    print(json.dumps(result, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
