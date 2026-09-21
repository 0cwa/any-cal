package org.anycal.android

import org.anycal.android.contacts.ContactsProjectionChecks
import org.anycal.android.contacts.ContactsReconciliationChecks
import org.anycal.android.contacts.ContactsSyncCallbackChecks
import org.anycal.android.BridgeRuntimeChecks
import org.anycal.android.sync.ProjectionStateChecks
import org.anycal.android.tasks.TasksOrgProviderChecks
import org.junit.Test

/** JUnit discovery wrapper for the repository's provider-free deterministic checks. */
class DeterministicChecksTest {
    @Test fun bridgeRuntime() = BridgeRuntimeChecks.runAll()
    @Test fun contactsProjection() = ContactsProjectionChecks.main(emptyArray())
    @Test fun contactsReconciliation() = ContactsReconciliationChecks.runAll()
    @Test fun contactsSync() = ContactsSyncCallbackChecks.runAll()
    @Test fun projectionState() = ProjectionStateChecks.main(emptyArray())
    @Test fun tasksOrgProvider() = TasksOrgProviderChecks.runAll()
}
