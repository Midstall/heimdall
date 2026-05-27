CREATE TABLE campaigns (
    id           TEXT PRIMARY KEY,
    dut          TEXT NOT NULL,
    chip_serial  TEXT,
    template     TEXT NOT NULL,
    state        TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX idx_campaigns_dut ON campaigns(dut);
CREATE INDEX idx_campaigns_state ON campaigns(state);

ALTER TABLE jobs ADD COLUMN campaign_id TEXT;
CREATE INDEX idx_jobs_campaign_id ON jobs(campaign_id);
