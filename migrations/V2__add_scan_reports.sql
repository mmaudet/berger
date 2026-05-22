-- Optional persistence of `berger scan` runs (PRD v1.1, milestone J3).
-- Written only when `berger scan --save-report` is passed; the scan is
-- otherwise strictly read-only and touches no table.
CREATE TABLE scan_reports (
    id INTEGER PRIMARY KEY,
    created_at TIMESTAMP NOT NULL,
    period_days INTEGER NOT NULL,
    messages_analyzed INTEGER NOT NULL,
    report_json TEXT NOT NULL,
    suggestions_json TEXT NOT NULL
);
