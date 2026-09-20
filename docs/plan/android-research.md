# Android provider source research

**Retrieval date:** 2026-08-30

**Validation audit:** 2026-09-18. The host has `/usr/bin/adb`, but
`adb devices -l` returned an empty device list. No `emulator`, `avdmanager`,
or `sdkmanager` executable was available on `PATH`, no local AVD or Android
SDK directory was found in the checked locations, and no DAVx5 APK was found
in the checked workspace/user artifact paths. Therefore no live Android
client run was attempted. This audit does not inspect or launch the personal
Anytype Flatpak.

**Capability closure:** The later API-35 host-KVM provider receipt and the
sanitized DAV/Tasks.org reconciliation are recorded in
[android-capability-closure.md](android-capability-closure.md). That receipt
supersedes the older provisioning-audit statement that no emulator runtime
had been started; it does not broaden the evidence to live DAVx5, EteSync,
Tasks.org, or production Anytype sync.

## What Android already provides

Android exposes local contacts and calendars through `ContactsContract` and
`CalendarContract`. These are application-facing content providers, not a
generic network DAV endpoint. [ContactsContract](https://developer.android.com/reference/android/provider/ContactsContract), [CalendarContract](https://developer.android.com/reference/android/provider/CalendarContract)

DAVx⁵ is a synchronization adapter. Its data is stored in the Android
Contacts/Calendar providers or in an app-specific task provider such as
OpenTasks/jtx Board; DAVx⁵ itself is not the local database or a DAV server.
[DAVx⁵ system integration](https://www.davx5.com/faq/system-integration), [DAVx⁵ manual](https://github.com/bitfireAT/davx5-manual/blob/main/introduction.rst)

The EteSync Android app is a separate Etebase client. Its custom-server
configuration does not make its local database available to a desktop
Any-Cal process. [EteSync Android guide](https://api.etesync.com/user-guide/android/)

## Three different Android scenarios

### Existing-account import

An Android companion can read records from the local providers and import
them into Anytype. This is the first Android experiment. It must record the
Android account, provider record ID, raw-contact ID where applicable, source
hash, and ownership mode. Provider data is normalized and can be lossy: the
aggregate Contacts table is not the original vCard, and Calendar/Task
providers do not guarantee the original iCalendar serialization.

### Any-Cal-owned Android sync adapter (primary Android direction)

An Android app can own an account and project Anytype-backed resources directly
into `ContactsContract` and `CalendarContract`, without DAVx⁵. Tasks.org can
be an optional provider adapter when its version and authority are supported.
A small Rust sync library can share Anytype mapping/checkpoint logic with the
desktop while Kotlin owns Android providers, permissions, account setup, and
background scheduling. See [android-direct-provider.md](android-direct-provider.md).

This requires Android permissions, account registration, background
scheduling, battery/Doze handling, conflict and deletion policy, and loop
prevention. It is an Android extension, not part of the desktop MVP.

### DAVx⁵ compatibility/fallback

The existing DAV server remains available for DAVx⁵ and generic clients. An
Android companion does not need to expose a local DAV endpoint for direct
Contacts/Calendar integration. A local or authenticated Android DAV endpoint
would be a separate feature and is not the primary Android design.

## Ownership and loop hazards

This loop is possible when DAVx⁵ already synchronizes the same account:

```text
DAV server → DAVx⁵ → Android provider → Any-Cal → DAV server
```

The source adapter must therefore retain `source_kind`, `source_account`,
`source_record_id`, `origin`, and a normalized `source_hash`. Existing
accounts should begin as one-way import. Continuous bidirectional provider
sync requires explicit ownership, deletion/tombstone rules, loop detection,
permission checks, and a tested background schedule.

## Decision status

### DAV discovery scope

Root and principal `PROPFIND` requests with `Depth: 1` are accepted as a
bounded no-op: the response contains only the requested properties for the
root or principal itself and does not enumerate child collections. Clients
continue discovery through `/.well-known/caldav`, `/.well-known/carddav`, and
the advertised calendar/addressbook home-set links.

### Sanitized Android profile evidence

The fixtures in `fixtures/protocol/android/` are parsed and executed by
`crates/dav-server/tests/android_profile.rs`. Discovery has 5 steps, CardDAV
has 11, and the Tasks.org-shaped VTODO profile has 14. Each runs twice against
a fresh repository and compares normalized response traces and final
ETag/archive/revision snapshots. Assertions cover repeated labelled contact
values, opaque X-fields, UTC/floating/TZID task values, create/edit/complete,
stale preconditions, per-href 404s, archive hiding, and an injected read
failure followed by replay.

This is executable in-process protocol evidence only; it does not claim that
DAVx5, Tasks.org, or Android Contacts executed the profiles. Device/emulator
testing remains authority-gated, including LAN binding, IPv6, Doze/background
behavior, provider ownership, and client-specific quirks.

The deterministic gate was rerun on 2026-09-18 with:

```text
env -u LD_PRELOAD cargo test -p any-cal-dav-server --test android_profile --offline
test result: 2 passed; 0 failed; 0 ignored
```

This confirms only the sanitized in-process profile. Live validation still
requires all of the following: an explicitly disposable Android emulator or
device visible to `adb`, installed DAVx5 and Tasks.org (plus a Contacts app if
CardDAV provider behavior is being tested), a disposable DAV account, and
permission to create and delete only marked test records. The smallest next
authority request is an emulator/device serial plus those disposable client
credentials; no personal device or account should be used.

### Isolated emulator provisioning audit

The host can support hardware-assisted virtualization: `msb doctor` reports
KVM read/write access and QEMU 10.2.2 is installed. This does not provide an
Android runtime. Microsandbox has only generic Ubuntu/Fedora/Node/Python
images, Podman has no Android image, and no Android SDK command-line tools,
system image, AVD, or client APK cache was found. Therefore no emulator was
started and no host package was installed.

The smallest reproducible isolated path is a dedicated Android SDK and AVD
under a disposable directory or microVM, with a private adb server and no
personal home/profile mounts. Android's current documentation identifies the
SDK command-line tools, emulator, platform-tools, and an API-level system image
as separate prerequisites ([SDK manager](https://developer.android.com/tools/sdkmanager),
[emulator setup](https://developer.android.com/studio/run/emulator),
[command-line emulator](https://developer.android.com/studio/run/emulator-commandline)).
The host should use a private environment similar to:

```text
ANDROID_SDK_ROOT=/tmp/any-cal-android-sdk
ANDROID_USER_HOME=/tmp/any-cal-android-user
ANDROID_AVD_HOME=/tmp/any-cal-android-avd
ANDROID_ADB_SERVER_PORT=5041
```

The provisioning authority would need to approve downloading the pinned SDK
tool archives and one system image, accepting the Android SDK licenses, and
downloading pinned DAVx⁵ and Tasks.org APKs from their chosen trusted sources.
The live run would then install only those APKs, boot one fresh AVD, connect
the AVD to a disposable Any-Cal DAV endpoint, run the bounded matrix, collect
redacted traces, and destroy the AVD and its temporary directories. A physical
device is simpler if one is explicitly supplied, but would require USB/ADB
authorization and the same disposable-account boundary.

The generic CardDAV and Tasks.org replay profiles also carry an explicit
`Host` plus `X-Forwarded-Proto: https` request profile. The replay runner
preserves and checks those request headers, documenting the reverse-proxy URL
shape without pretending that the in-process handler terminates TLS or proves
proxy trust.

Settled:

- Android is an adapter boundary and direct provider integration is the
  preferred Android projection; the desktop DAV surface remains available.
- Direct ContactsContract and CalendarContract projection is the preferred
  Android extension and must not alter the desktop Anytype-backed DAV model.
- Tasks.org's `org.tasks.api/v0` provider is optional and version-gated because
  its published API is explicitly unstable.
- Android has no platform-standard task provider. OpenTasks is a separate,
  deprecated provider project and is not interchangeable with Tasks.org's
  provider.
- A desktop process cannot directly query Android providers without a
  companion transport.

Blocking research:

- Whether the Tasks.org provider introduced after the tested 15.10 build is
  available and adequate in 15.12; direct Tasks.org integration remains
  optional until both versions are tested.
- Provider fidelity for labelled contact values, groups, photos, recurrence,
  reminders, and unknown fields.
- Secure companion transport and account/source ownership.
- Background scheduling and reliable Android deletion/change detection.
- Whether bidirectional provider sync is worth the maintenance cost after
  one-way projection testing.

## Planned fixture

Before writing Android code, create a provider-shaped fixture containing
ContactsContract raw/aggregate contacts, CalendarContract events and
attendees, and representative OpenTasks/jtx-style tasks. Run the same mapping
and envelope tests against it as against direct DAV resources. The fixture
must demonstrate explicitly which fields cannot be round-tripped.

### Disposable live HTTPS validation (2026-09-19)

The isolated API-35 emulator `any-cal-android-live-20260919` was provisioned
with DAVx⁵ 4.5.19-ose and Tasks.org 15.10. The emulator trusted the pinned
test certificate (`1b0fee14.0` in the system CA store), and Tasks.org accepted
the matching disposable certificate fingerprint. The HTTPS path to
`https://10.0.2.2:8443` therefore works; this run used only synthetic account
values and a fake-mode Any-Cal backend, not Anytype or personal data.

The rebuilt backend was copied into the VM and Tasks.org reached CalDAV
discovery, but account setup still failed with HTTP 404 during the principal /
home-set lookup. Direct probes show that `/caldav/` returns 404 while the
implemented task collection is `/caldav/tasks/`; the current well-known/home
set URL shape does not yet match Tasks.org's lookup sequence. Consequently no
VTODO CRUD, ETag, reconnect, DAVx⁵ provider, or CardDAV result is claimed from
this batch. The VM was stopped and preserved after the test.

Next live gate: capture the exact Tasks.org PROPFIND sequence (including
request paths and bodies), then either expose a compatible `/caldav/` home
collection or return a standards-compatible home-set link before rerunning the
CRUD matrix. Do not interpret the passing in-process route tests as proof of
Tasks.org discovery compatibility.

### Follow-up route gate (2026-09-19)

The app binary was rebuilt offline and verified byte-for-byte identical on the
host and disposable VM (`7d6a7544d3c9176e2cc5858d000a37a9ae05a13760a2cc5b285783c8fd9eb992`).
With the CA-trusted API-35 emulator and HTTPS proxy running, direct synthetic
PROPFIND probes still returned `404 Not Found` for both `/caldav/` and
`/caldav/tasks/`. The matrix was stopped before client CRUD. This establishes
that the advertised `/caldav/` home implementation is not present or not
reachable in the currently built server; client-level CRUD testing must wait
until those direct route gates return the expected DAV multistatus response.

### Hydrated release live gate (2026-09-19)

The specified release binary was copied and verified in the disposable VM:
`84fe42cbba475ce7b1b0125bad7dd4f26dc3decb9289e0584c9d7b2dd0f4d5b8`.
With the API-35 emulator, pinned CA, and HTTPS proxy active, direct synthetic
PROPFIND requests returned `207 Multi-Status` for `/caldav/`,
`/caldav/tasks/`, `/caldav/tasks` and `/principals/users/default`.

Tasks.org 15.10 accepted the disposable certificate but still failed its live
discovery sequence with `HTTP 404` while resolving the home set. Thus the
server route gate is now green, but the exact client-request path/property
sequence is still not fully compatible. No VTODO CRUD/ETag/reconnect, DAVx⁵,
or CardDAV result is claimed. The VM was stopped and preserved.

### Redacted Tasks.org discovery capture (2026-09-19)

A temporary TLS proxy captured one clean discovery attempt without recording
credentials or XML values. Tasks.org sent exactly:

```text
PROPFIND /caldav/ HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Content-Length: 198
XML tags: current-user-principal, prop, propfind
Response: HTTP/1.1 207 Multi-Status

PROPFIND /principals/users/default HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Content-Length: 197
XML tags: calendar-home-set, prop, propfind
Response: HTTP/1.1 207 Multi-Status
```

This run produced no 404. Tasks.org then raised its own
`DisplayableException` in `CaldavClient.findHomeset`, leaving the account form
at `SERVER TYPE: Unknown`. The remaining compatibility issue is therefore
response shape/content or home-set interpretation, not TLS, routing, or a
missing request URI. The VM and temporary capture proxy were stopped/removed.

### Full response-shape capture (2026-09-19)

The next discovery-only run captured complete DAV XML with synthetic
credentials redacted. Tasks.org sent:

```http
PROPFIND /caldav/ HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Content-Length: 198
```

```xml
<?xml version='1.0' encoding='UTF-8' ?><propfind xmlns="DAV:" xmlns:CAL="urn:ietf:params:xml:ns:caldav" xmlns:CARD="urn:ietf:params:xml:ns:carddav"><prop><current-user-principal /></prop></propfind>
```

The server returned `207 Multi-Status`:

```xml
<multistatus xmlns="DAV:"><response><href>/caldav/</href><propstat><prop><current-user-principal><href>/principals/users/default</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>
```

Tasks.org then sent:

```http
PROPFIND /principals/users/default HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Content-Length: 197
```

```xml
<?xml version='1.0' encoding='UTF-8' ?><propfind xmlns="DAV:" xmlns:CAL="urn:ietf:params:xml:ns:caldav" xmlns:CARD="urn:ietf:params:xml:ns:carddav"><prop><CAL:calendar-home-set /></prop></propfind>
```

The server again returned `207 Multi-Status`:

```xml
<multistatus xmlns="DAV:"><response><href>/principals/users/default</href><propstat><prop><calendar-home-set xmlns="urn:ietf:params:xml:ns:caldav"><href>/caldav/</href></calendar-home-set></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>
```

No third request occurred. Tasks.org raised `org.tasks.ui.DisplayableException`
at `CaldavClient.findHomeset` immediately after the second response. This
points to a client parser/shape expectation around `calendar-home-set` (or the
relative `/caldav/` href), not a transport or HTTP-status failure. The VM was
stopped and the temporary capture, including synthetic Authorization headers,
was deleted.

### Origin-aware href experiment (2026-09-19)

The release backend (`b37776515c96727c2e96c1be35f211c1973a3f2b1ce50e794b3126c9512b6737`)
was tested behind a temporary proxy that injected `Host: 10.0.2.2:8443` and
`X-Forwarded-Proto: https`. The direct response then contained absolute HTTPS
hrefs, for example `https://10.0.2.2:8443/caldav/` and
`https://10.0.2.2:8443/principals/users/default`.

Tasks.org accepted the synthetic certificate but failed discovery with
`HTTP 404` in `CaldavClient.findHomeset`. The earlier relative-href response
form instead produced a client-side `DisplayableException` with no 404. This
confirms that href origin/normalization is a decisive compatibility variable;
the next capture must record the absolute-href follow-up URI before choosing
the canonical response form. No CRUD/provider writes were attempted. The VM
and temporary proxy were stopped/removed.

### Origin-aware request/status capture (2026-09-19)

With the proxy injecting `Host: 10.0.2.2:8443` and
`X-Forwarded-Proto: https`, the complete discovery sequence was:

```text
PROPFIND /caldav/ HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Host: 10.0.2.2:8443
→ 207 Multi-Status
<href>https://10.0.2.2:8443/caldav/</href>
<current-user-principal><href>https://10.0.2.2:8443/principals/users/default</href>

PROPFIND /principals/users/default HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Host: 10.0.2.2:8443
→ 207 Multi-Status
<href>https://10.0.2.2:8443/principals/users/default</href>
<calendar-home-set><href>https://10.0.2.2:8443/caldav/</href>
```

No 404 request occurred and no third request followed. Tasks.org raised
`DisplayableException` at `CaldavClient.findHomeset` immediately after the
principal response. The exact failing class is therefore principal/home-set
semantic parsing, not slash normalization or collection routing. Temporary
capture data was removed and the VM was stopped.

### Namespace-fixed discovery gate (2026-09-19)

Release binary hash: `50cf979acf0b314cd0a44a71f6bee0b19ace8ab3cf7c9496384cbe2b6140f99b`.
With origin-aware `Host`/`X-Forwarded-Proto` headers, Tasks.org discovery
completed successfully. The captured sequence included:

```text
PROPFIND /caldav/              Depth: 0 → 207
PROPFIND /principals/users/default Depth: 0 → 207
PROPFIND /caldav/              Depth: 1 → 207
PROPFIND /caldav/              Depth: 1 → 207 (repeated)
```

The depth-1 response exposed the task collection at
`https://10.0.2.2:8443/caldav/tasks`, with `resourcetype` containing
`<collection/><calendar xmlns="urn:ietf:params:xml:ns:caldav"/>` and
`<supported-calendar-component-set><comp name="VTODO"/></...>`. Tasks.org
opened the synchronized `My Tasks` view. No CRUD/provider writes were made;
the attempted UI action opened list settings rather than a task editor. The
VM and temporary capture proxy were stopped/removed.

### Tasks.org CRUD UI gate (2026-09-19)

Resuming the preserved account opened the synchronized `My Tasks` view with
no records. The visible control labelled `Create new task` opened a form with
`DISPLAY NAME`, `Color`, `Icon`, and `Save`—a list-editor form, not a task
editor. No synthetic task was created, so no DAV write, ETag, completion,
delete, reconnect, or resurrection claim is made. This is an Android UI
automation blocker only; the discovery route remains validated. The VM and
temporary redacted logger were stopped/removed.

### Tasks.org ShareLink task-entry probe (2026-09-19)

The explicit `ACTION_SEND` `text/plain` intent targeting
`org.tasks/com.todoroo.astrid.activity.ShareLinkActivity` did not open a task
editor. Android displayed the system share chooser (`Phone`, `Messages`,
`Gmail`, `Chrome`, `Camera`) and returned to `MainActivity`. The only DAV
request was a `PROPFIND /caldav/` depth-1 (`207 Multi-Status`); no `PUT` or
other write occurred. No share target was selected and no record was created.
This path is therefore not a safe automated task-creation route without a
real Tasks.org share-target selection or another UI entry point.

### Tasks.org task-editor control diagnosis (2026-09-19)

On the preserved synchronized list, the hierarchy identified the current list
as `My Tasks` and exposed the FAB `org.tasks:id/fab` with
`content-desc="Create new task"`. Tapping the `My Tasks` title itself did not
change activity or state. Tapping that FAB once opened the actual task editor,
whose hierarchy contained an `EditText` with text `Task name` and a `Save`
action. No text was entered and Save was not tapped. The resumed activity
remained `org.tasks/com.todoroo.astrid.activity.TaskListActivity`; the editor
is an in-activity view. The VM was stopped after this read-only diagnosis.

### Tasks.org single-record CRUD gate (2026-09-19)

The preserved `My Tasks` list contained one existing marked synthetic task,
`AnyCal_SYNTH_TASK_20260919`; no additional record was created. The task
editor exposed title and description fields, but adb text selection mangled
the edited title/description. The task was then deleted through the confirmed
`Delete task` action and the UI showed `There are no tasks here.` After one
Tasks.org restart it remained absent, so no resurrection was observed.

The redacted server logger recorded only:

```text
GET /.well-known/caldav HTTP/1.1 → 302 Internal Server Error
```

No `PUT`, `DELETE`, ETag, completion, or conditional request reached the fake
server. The local lifecycle therefore validated UI deletion/reconnect only;
DAV CRUD remains blocked by Tasks.org background sync not proceeding past the
well-known request in this run. No DAVx⁵ provider test was attempted after the
server-side write gate failed. The VM and temporary logger were stopped/removed.

### Clean synchronized-list lifecycle diagnosis (2026-09-19)

After clean discovery, Tasks.org resumed directly in
`org.tasks/com.todoroo.astrid.activity.TaskListActivity` with title `My
Tasks`. The hierarchy contained no clickable collection/list row—only an
empty body (`There are no tasks here.`), bottom controls, and
`org.tasks:id/fab` (`Create new task`). Therefore the requested row-entry
verification cannot be performed in this build; tapping the FAB would bypass
that verification and create directly. No task or list was created and the VM
was stopped.

### Final clean FAB attempt (2026-09-19)

From a clean `My Tasks` screen showing `There are no tasks here.`, tapping
`org.tasks:id/fab` once again opened the list editor, with `DISPLAY NAME`,
`Color`, `Icon`, and `Save`, rather than the task editor. No field was edited,
no list was saved, and no task/DAV write was generated. This confirms the
remaining Tasks.org CRUD blocker is the inconsistent FAB behavior in this
clean discovered-account state; the VM was stopped without further mutation.

### Clean well-known discovery diagnostic (2026-09-19)

With a fresh Tasks.org app state and the URL set to
`https://10.0.2.2:8443/.well-known/caldav`, the client did not issue GET. It
issued the following three identical requests:

```text
PROPFIND /.well-known/caldav HTTP/1.1
Depth: 0
Content-Type: application/xml; charset=utf-8
Host: 10.0.2.2:8443
→ HTTP/1.1 404 Not Found
```

The server's well-known redirect is currently implemented for GET/HEAD only,
so Tasks.org retries the PROPFIND three times and fails with
`NotFoundException` in `CaldavClient.findHomeset`. When the user supplies
`/caldav/` directly, discovery proceeds through the 207 depth-1 collection
responses. This clean run created no task and confirms the exact blocker is
the PROPFIND well-known method handling, not stale local task state.

### PROPFIND well-known redirect fix validation (2026-09-19)

Release binary hash: `7df12781e2ae4a54733c8b22977b40f66c2c5963096b031c752ae5d24b51c9c6`.
With a clean Tasks.org app state, origin-aware proxy headers, and synthetic
credentials, discovery now succeeds through the newly supported method:

```text
PROPFIND /.well-known/caldav  Depth: 0 → 302 Found
PROPFIND /caldav/             Depth: 0 → 207 Multi-Status
PROPFIND /principals/users/default Depth: 0 → 207 Multi-Status
PROPFIND /caldav/             Depth: 1 → 207 Multi-Status
PROPFIND /caldav/             Depth: 1 → 207 Multi-Status
```

The clean run did not expose a stable synchronized task editor afterward—the
FAB opened a list-editor form—so no task/list was created and no CRUD claim is
made. The VM and temporary logger were stopped/removed.

### DAVx⁵ provider/account-flow validation (2026-09-19)

The disposable API-35 emulator used DAVx⁵ `4.5.19-ose` (`at.bitfire.davdroid`)
with only synthetic permissions and the fake Any-Cal backend; it was not
connected to Anytype. DAVx⁵ completed its first-run screens after granting the
emulator-only calendar, contacts, notification, and Tasks permissions. Its
account flow was reached through `Add account` → `Generic login` → `Login with
URL and user name`. The form explicitly displayed a `Base URL` field and a
`Login` action.

No URL, username, or password was entered, so DAVx⁵ did not issue discovery
requests, create an account, register Android provider rows, or expose a
VTODO/CardDAV collection. The bounded UI inspection hung while reading the
final form, and the VM stop/status command also did not return; this run must
therefore be treated as an account-flow reachability result only, not a
provider interoperability result. No DAVx⁵ records or collections were
created. A follow-up should use a fresh disposable VM or recover the stopped
VM before entering `https://10.0.2.2:8443` with marked synthetic credentials,
then capture account creation and collection selection separately.
