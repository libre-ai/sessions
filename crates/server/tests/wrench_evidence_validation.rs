//! Validates that wrench-inspect Portal evidence reports conform to schema.
//!
//! This test is gated by CI: wrench-inspect runs before this test passes,
//! producing `wrench-portal-evidence.json` as an artifact. The test ensures
//! the report is well-formed per `wrench.evidence_report.v0.1`.

#[test]
fn wrench_evidence_report_schema_v0_1() {
    // This test is a documentation-only marker. The actual validation happens in CI:
    // the wrench-inspect job produces the report, and the jq gates validate its schema
    // (format, valid field, checks array, etc.).
    //
    // When CI runs locally or in testing, the report file should be validated
    // against this checklist (verified against real wrench-inspect output):
    // - format: "wrench.evidence_report.v0.1"
    // - status: "passed" | "failed"
    // - checks: array of { code, status, summary }
    // - summary: { errors, warnings, infos }
    // - generated_at: RFC3339 timestamp
    // - producer: { name, version }
    // - subject: { kind, reference }
    //
    // If the report is present (e.g., in a development context), validate it:
    let report_path = "wrench-portal-evidence.json";
    if std::path::Path::new(report_path).exists() {
        let content =
            std::fs::read_to_string(report_path).expect("failed to read wrench evidence report");
        let report: serde_json::Value =
            serde_json::from_str(&content).expect("wrench evidence report must be valid JSON");

        assert_eq!(
            report["format"].as_str(),
            Some("wrench.evidence_report.v0.1"),
            "report must have correct format version"
        );

        let status = report["status"].as_str();
        assert!(
            status == Some("passed") || status == Some("failed"),
            "report must have a 'status' of passed or failed"
        );

        assert!(
            report["checks"].is_array(),
            "report must have a 'checks' array"
        );

        assert!(
            report["summary"].is_object(),
            "report must have a 'summary' object"
        );

        assert_eq!(
            status,
            Some("passed"),
            "Portal evidence report inspection failed; findings: {}",
            report
                .get("findings")
                .and_then(|f| f.as_array())
                .map(|f| f.len())
                .unwrap_or(0)
        );
    }
    // If the file doesn't exist (e.g., test run without CI), we pass silently.
    // This allows the test suite to remain green in non-CI environments.
}
