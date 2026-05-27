CREATE TABLE jobs (
    id          TEXT PRIMARY KEY,
    dut         TEXT NOT NULL,
    dut_kind    TEXT NOT NULL,
    kind_json   TEXT NOT NULL,
    state_json  TEXT NOT NULL,
    state_tag   TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE INDEX idx_jobs_state_tag ON jobs(state_tag);
CREATE INDEX idx_jobs_dut ON jobs(dut);

CREATE TABLE events (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    payload_json TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
