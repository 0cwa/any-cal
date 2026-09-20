#!/usr/bin/env python3
"""Offline parser/contract test using a deterministic fake adb executable."""

import json
import os
import stat
import subprocess
import tempfile
from pathlib import Path


FAKE_ADB = r'''#!/bin/sh
case "$*" in
  *"devices"*) printf 'List of devices attached\nprobe-5555\tdevice\n' ;;
  *"getprop ro.product.manufacturer"*) echo Google ;;
  *"getprop ro.product.model"*) echo Probe ;;
  *"getprop ro.build.version.release"*) echo 15 ;;
  *"getprop ro.build.version.sdk"*) echo 35 ;;
  *"getprop sys.boot_completed"*) echo 1 ;;
  *"resolve-content-provider --brief com.android.contacts"*) echo 'com.android.providers.contacts/.ContactsProvider2' ;;
  *"resolve-content-provider --brief com.android.calendar"*) echo 'com.android.providers.calendar/.CalendarProvider2' ;;
  *"resolve-content-provider --brief org.tasks.api"*) echo 'No providers found' ;;
  *"dumpsys package providers"*) exit 0 ;;
  *"pm path org.tasks"*) exit 1 ;;
  *"pm path com.anycal.app"*) exit 1 ;;
  *"check-permission"*) echo denied ;;
  *) exit 1 ;;
esac
'''


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        fake = Path(directory) / "adb"
        fake.write_text(FAKE_ADB)
        fake.chmod(fake.stat().st_mode | stat.S_IXUSR)
        completed = subprocess.run(
            ["python3", str(Path(__file__).with_name("probe.py")), "--adb", str(fake)],
            check=False,
            capture_output=True,
            text=True,
            env={**os.environ, "PATH": directory + os.pathsep + os.environ.get("PATH", "")},
        )
        assert completed.returncode == 0, completed.stderr
        report = json.loads(completed.stdout)
        assert report["environment"] == "available"
        assert report["device"]["api_level"] == "35"
        assert report["providers"]["contacts"]["available"] is True
        assert report["providers"]["calendar"]["owner_package"] == "com.android.providers.calendar"
        assert report["providers"]["tasks_org"]["resolution_status"] == "absent"
        assert report["packages"]["tasks_org"]["installed"] is False
        assert report["account_provider_boundary"]["account_enumeration"] == "not_performed"
    print("android-probe fake-device validation: PASS")


if __name__ == "__main__":
    main()
