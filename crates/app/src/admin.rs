use super::{add_correlation, no_store, AppGeneric};
use any_cal_anytype_adapter::AnytypeTransport;
use any_cal_observability::{
    AuditReadbackQuery, AuditSummary, Correlation, ErrorCategory, Event, MAX_AUDIT_READBACK,
};
use std::path::{Path, PathBuf};

impl<T: AnytypeTransport> AppGeneric<T> {
    fn sync_export_path(&self) -> Option<PathBuf> {
        self.config
            .sync_checkpoint
            .as_deref()
            .map(Path::new)
            .map(|path| path.with_extension("export"))
    }

    fn admin_authorized(&self, request: &any_cal_dav_server::Request) -> bool {
        self.config
            .auth_credential
            .as_deref()
            .is_some_and(|expected| self.authorized_credential(request, expected))
    }

    fn sync_admin_error(
        status: u16,
        message: &'static str,
        correlation: Option<&Correlation>,
    ) -> any_cal_dav_server::Response {
        let mut headers = vec![("Content-Type".into(), "application/json".into())];
        no_store(&mut headers);
        if let Some(correlation) = correlation {
            add_correlation(&mut headers, correlation);
        }
        if status == 405 {
            headers.push(("Allow".into(), "GET, POST".into()));
        }
        any_cal_dav_server::Response {
            status,
            headers,
            body: format!("{{\"status\":\"error\",\"error\":\"{message}\"}}").into_bytes(),
        }
    }

    fn sync_admin_receipt(
        operation: &'static str,
        receipt: &any_cal_sync::ExportReceipt,
        correlation: &Correlation,
    ) -> any_cal_dav_server::Response {
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Cache-Control".into(), "no-store".into()),
        ];
        add_correlation(&mut headers, correlation);
        any_cal_dav_server::Response {
            status: 200,
            headers,
            body: format!(
                "{{\"status\":\"ok\",\"operation\":\"{operation}\",\"format\":\"{}\",\"export_version\":{},\"state_generation\":{},\"observed\":{},\"pending\":{},\"tombstones\":{}}}",
                receipt.format,
                receipt.export_version,
                receipt.state_generation,
                receipt.observed,
                receipt.pending,
                receipt.tombstones,
            )
            .into_bytes(),
        }
    }

    fn sync_admin_audit_readback(
        summaries: Vec<AuditSummary>,
        correlation: &Correlation,
    ) -> any_cal_dav_server::Response {
        let next_after = summaries.last().map(|summary| summary.sequence);
        let body = serde_json::json!({
            "status": "ok",
            "count": summaries.len(),
            "next_after": next_after,
            "events": summaries,
        });
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Cache-Control".into(), "no-store".into()),
        ];
        add_correlation(&mut headers, correlation);
        any_cal_dav_server::Response {
            status: 200,
            headers,
            body: serde_json::to_vec(&body).expect("audit readback response serializes"),
        }
    }

    fn parse_audit_readback_query(path: &str) -> Result<AuditReadbackQuery, &'static str> {
        let Some((_, query)) = path.split_once('?') else {
            return Ok(AuditReadbackQuery {
                after_sequence: None,
                limit: MAX_AUDIT_READBACK,
            });
        };
        let mut after_sequence = None;
        let mut limit = None;
        for component in query.split('&') {
            let Some((key, value)) = component.split_once('=') else {
                return Err("malformed_query");
            };
            if value.is_empty() {
                return Err("malformed_query");
            }
            match key {
                "after" if after_sequence.is_none() => {
                    after_sequence = Some(value.parse().map_err(|_| "malformed_query")?);
                }
                "limit" if limit.is_none() => {
                    limit = Some(value.parse().map_err(|_| "malformed_query")?);
                }
                _ => return Err("unsupported_query"),
            }
        }
        let query = AuditReadbackQuery {
            after_sequence,
            limit: limit.unwrap_or(MAX_AUDIT_READBACK),
        };
        query.validate().map_err(|_| "limit_out_of_bounds")?;
        Ok(query)
    }

    pub(super) fn handle_sync_admin(
        &mut self,
        request: any_cal_dav_server::Request,
        correlation: Correlation,
    ) -> any_cal_dav_server::Response {
        if !self.admin_authorized(&request) {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Auth),
                "operation=denied",
            ));
            return Self::sync_admin_error(401, "unauthorized", Some(&correlation));
        }
        let (route, _) = request.path.split_once('?').unwrap_or((&request.path, ""));
        if route == "/admin/sync/audit" {
            if request.method != "GET" {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=audit_readback status=method_not_allowed",
                ));
                return Self::sync_admin_error(405, "method_not_allowed", Some(&correlation));
            }
            if !request.body.is_empty() {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=audit_readback status=request_body_not_allowed",
                ));
                return Self::sync_admin_error(400, "request_body_not_allowed", Some(&correlation));
            }
            let query = match Self::parse_audit_readback_query(&request.path) {
                Ok(query) => query,
                Err(error) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        Some(ErrorCategory::Protocol),
                        &format!("operation=audit_readback status={error}"),
                    ));
                    return Self::sync_admin_error(400, error, Some(&correlation));
                }
            };
            let Some(audit) = self.audit.as_ref() else {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Recovery),
                    "operation=audit_readback status=audit_unavailable",
                ));
                return Self::sync_admin_error(503, "audit_unavailable", Some(&correlation));
            };
            match audit.readback(query) {
                Ok(summaries) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        None,
                        "operation=audit_readback status=ok",
                    ));
                    return Self::sync_admin_audit_readback(summaries, &correlation);
                }
                Err(_) => {
                    self.record_admin_audit(correlation.event(
                        "recovery.admin",
                        Some(ErrorCategory::Recovery),
                        "operation=audit_readback status=failed",
                    ));
                    return Self::sync_admin_error(503, "audit_unavailable", Some(&correlation));
                }
            }
        }
        let operation = match request.path.as_str() {
            "/admin/sync" | "/admin/sync/capabilities" if request.method == "GET" => {
                let configured = self.sync.is_some();
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    None,
                    &format!("operation=capabilities status=ok configured={configured}"),
                ));
                let mut headers = vec![
                    ("Content-Type".into(), "application/json".into()),
                    ("Cache-Control".into(), "no-store".into()),
                ];
                add_correlation(&mut headers, &correlation);
                return any_cal_dav_server::Response {
                    status: 200,
                    headers,
                    body: format!(
                        "{{\"status\":\"ok\",\"export\":{},\"restore\":{},\"format\":\"any-cal.sync-export\",\"export_version\":1}}",
                        configured, configured
                    )
                    .into_bytes(),
                };
            }
            "/admin/sync/export" if request.method == "POST" => "export",
            "/admin/sync/restore" if request.method == "POST" => "restore",
            "/admin/sync"
            | "/admin/sync/capabilities"
            | "/admin/sync/export"
            | "/admin/sync/restore" => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=admin status=method_not_allowed",
                ));
                return Self::sync_admin_error(405, "method_not_allowed", Some(&correlation));
            }
            _ => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Protocol),
                    "operation=admin status=not_found",
                ));
                return Self::sync_admin_error(404, "not_found", Some(&correlation));
            }
        };
        if !request.body.is_empty() {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Protocol),
                &format!("operation={operation} status=request_body_not_allowed"),
            ));
            return Self::sync_admin_error(400, "request_body_not_allowed", Some(&correlation));
        }
        let Some(path) = self.sync_export_path() else {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Recovery),
                &format!("operation={operation} status=sync_checkpoint_unavailable"),
            ));
            return Self::sync_admin_error(503, "sync_checkpoint_unavailable", Some(&correlation));
        };
        let Some(sync) = self.sync.as_mut() else {
            self.record_admin_audit(correlation.event(
                "recovery.admin",
                Some(ErrorCategory::Recovery),
                &format!("operation={operation} status=sync_checkpoint_unavailable"),
            ));
            return Self::sync_admin_error(503, "sync_checkpoint_unavailable", Some(&correlation));
        };
        let result = if operation == "export" {
            sync.export_to(&path)
        } else {
            sync.restore_export(&path)
        };
        match result {
            Ok(receipt) => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    None,
                    &format!("operation={operation} status=ok"),
                ));
                Self::sync_admin_receipt(operation, &receipt, &correlation)
            }
            Err(_) => {
                self.record_admin_audit(correlation.event(
                    "recovery.admin",
                    Some(ErrorCategory::Recovery),
                    &format!("operation={operation} status=failed"),
                ));
                Self::sync_admin_error(500, "sync_operation_failed", Some(&correlation))
            }
        }
    }

    /// Persist admin-operation events when the optional durable audit journal
    /// is configured. Admin responses retain their operation result even if
    /// the diagnostic sink is degraded; the writer health surface reports
    /// that failure separately.
    pub(super) fn record_admin_audit(&mut self, event: Event) {
        self.events.push(event.clone());
        if let Some(audit) = self.audit.as_ref() {
            let _ = audit.append(event);
        }
    }
}
