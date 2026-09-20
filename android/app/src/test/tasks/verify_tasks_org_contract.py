#!/usr/bin/env python3
"""Dependency-free, no-provider CRUD checks for the Tasks.org adapter contract."""

import json
from pathlib import Path


ROOT = Path(__file__).with_name("tasks_org_contract_fixture.json")
TASKS_DIR = ROOT.parents[2] / "main" / "kotlin" / "org" / "anycal" / "android" / "tasks"


def probe_decision(case, fixture):
    if case["provider_package"] != "org.tasks":
        return "unsupported"
    allowed = fixture["allow_list"].get(case["provider_package"], [])
    if case["version_code"] not in allowed:
        return "unsupported"
    return "supported" if case["read_granted"] and case["write_granted"] else "unsupported"


def project_columns(task):
    return {
        "title": task["title"],
        "notes": task["notes"],
        "due_date": task["due_date"],
        "due_all_day": task["due_all_day"],
        "start_date": task["start_date"],
        "start_all_day": task["start_all_day"],
        "completed_at": task["completed_at"],
        "recurrence": task["recurrence"],
        "list_id": task["list_provider_id"],
        "parent_id": task["parent_provider_id"],
    }


def main():
    fixture = json.loads(ROOT.read_text())
    assert fixture["base_uri"] == "content://org.tasks.api/v0"
    assert fixture["authority"] == "org.tasks.api"
    assert fixture["api_version"] == "v0"
    assert fixture["permissions"] == [
        "org.tasks.permission.READ_TASKS",
        "org.tasks.permission.WRITE_TASKS",
    ]

    for case in fixture["probe_cases"]:
        assert probe_decision(case, fixture) == case["expected"], case["name"]

    task = fixture["task"]
    projected = project_columns(task)
    assert projected == fixture["expected_columns"]
    for forbidden in fixture["forbidden_columns"]:
        assert forbidden not in projected
    assert task["canonical_id"] != str(projected.get("list_id"))
    assert isinstance(task["canonical_id"], str)
    assert isinstance(projected["list_id"], int)
    assert isinstance(projected["parent_id"], int)

    mapping_source = (TASKS_DIR / "TasksOrgMapping.kt").read_text()
    for column in fixture["expected_columns"]:
        assert f'"{column}"' in mapping_source, column
    adapter_source = (TASKS_DIR / "TasksOrgAdapter.kt").read_text()
    assert 'const val AUTHORITY = "org.tasks.api"' in adapter_source
    assert 'const val API_VERSION = "v0"' in adapter_source
    assert 'const val READ_PERMISSION = "org.tasks.permission.READ_TASKS"' in adapter_source
    assert 'const val WRITE_PERMISSION = "org.tasks.permission.WRITE_TASKS"' in adapter_source
    for operation in ("insert(", "update(", "delete("):
        assert operation not in adapter_source, operation
    provider_source = (TASKS_DIR / "TasksOrgProvider.kt").read_text()
    for operation in ("resolver.insert(", "resolver.update(", "resolver.delete("):
        assert operation in provider_source, operation
    manifest = (ROOT.parents[3] / "src" / "main" / "AndroidManifest.xml").read_text()
    assert 'android:name="org.tasks.permission.READ_TASKS"' in manifest
    assert 'android:name="org.tasks.permission.WRITE_TASKS"' in manifest
    assert 'android:authorities="org.tasks.api"' in manifest
    assert "151202" in adapter_source
    print("Tasks.org contract checks passed: no provider access or CRUD performed")


if __name__ == "__main__":
    main()
