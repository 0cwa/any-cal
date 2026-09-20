package org.anycal.android.calendar

/** Regression check for CalendarContract sync-adapter write URI ownership. */
object CalendarContractStoreUriAcceptanceTest {
    fun runAll() {
        val parameters = CalendarContractStore.syncAdapterQueryParameters("user", "org.anycal")
        check(parameters.size == 3)
        check(parameters["caller_is_syncadapter"] == "true")
        check(parameters["account_name"] == "user")
        check(parameters["account_type"] == "org.anycal")
    }
}
