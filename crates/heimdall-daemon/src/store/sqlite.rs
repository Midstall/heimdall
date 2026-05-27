//! SQLite-backed JobStore. Behind the `sqlite` cargo feature.

use async_trait::async_trait;
use chrono::Utc;
use heimdall_core::{DutId, DutKind};
use sqlx::Row;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Pool, Sqlite};
use std::path::Path;
use std::str::FromStr;

use crate::error::{DaemonError, Result};
use crate::store::JobStore;
use crate::types::{
    Campaign, CampaignId, CampaignState, CampaignTemplate, Event, EventId, Job, JobFilter, JobId,
    JobKind, JobState, NewJob,
};

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

pub struct SqliteJobStore {
    pool: Pool<Sqlite>,
}

impl SqliteJobStore {
    /// Open a sqlite store at the given file path. Creates the file if missing
    /// and runs migrations.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self> {
        let url = format!("sqlite://{}", path.as_ref().display());
        let opts = SqliteConnectOptions::from_str(&url)
            .map_err(DaemonError::Sqlx)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(opts)
            .await
            .map_err(DaemonError::Sqlx)?;
        MIGRATOR.run(&pool).await.map_err(DaemonError::Migrate)?;
        Ok(Self { pool })
    }

    /// Open an in-memory store for tests. Each call creates an isolated db.
    pub async fn open_in_memory() -> Result<Self> {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:")
            .map_err(DaemonError::Sqlx)?
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1) // memory db tied to a single connection
            .connect_with(opts)
            .await
            .map_err(DaemonError::Sqlx)?;
        MIGRATOR.run(&pool).await.map_err(DaemonError::Migrate)?;
        Ok(Self { pool })
    }
}

fn dut_kind_str(k: &DutKind) -> String {
    // Reuse the kebab-case serde representation by serializing as JSON
    // and stripping the quotes.
    serde_json::to_string(k)
        .unwrap_or_else(|_| "\"unknown\"".into())
        .trim_matches('"')
        .to_string()
}

fn dut_kind_from_str(s: &str) -> Result<DutKind> {
    serde_json::from_str(&format!("\"{}\"", s))
        .map_err(|e| DaemonError::Config(format!("dut_kind `{s}`: {e}")))
}

fn row_to_job(row: &sqlx::sqlite::SqliteRow) -> Result<Job> {
    let id_str: String = row.try_get("id").map_err(DaemonError::Sqlx)?;
    let dut: String = row.try_get("dut").map_err(DaemonError::Sqlx)?;
    let dut_kind_s: String = row.try_get("dut_kind").map_err(DaemonError::Sqlx)?;
    let kind_json: String = row.try_get("kind_json").map_err(DaemonError::Sqlx)?;
    let state_json: String = row.try_get("state_json").map_err(DaemonError::Sqlx)?;
    let created_at: String = row.try_get("created_at").map_err(DaemonError::Sqlx)?;
    let updated_at: String = row.try_get("updated_at").map_err(DaemonError::Sqlx)?;

    let id = JobId(
        uuid::Uuid::parse_str(&id_str)
            .map_err(|e| DaemonError::Config(format!("invalid job id `{id_str}`: {e}")))?,
    );
    let kind: JobKind = serde_json::from_str(&kind_json)?;
    let state: JobState = serde_json::from_str(&state_json)?;
    let dut_kind = dut_kind_from_str(&dut_kind_s)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
        .map_err(|e| DaemonError::Config(format!("created_at: {e}")))?
        .with_timezone(&Utc);
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
        .map_err(|e| DaemonError::Config(format!("updated_at: {e}")))?
        .with_timezone(&Utc);
    let campaign_id_str: Option<String> = row.try_get("campaign_id").ok();
    let campaign = match campaign_id_str {
        Some(s) if !s.is_empty() => {
            Some(CampaignId(uuid::Uuid::parse_str(&s).map_err(|e| {
                DaemonError::Config(format!("invalid campaign id `{s}`: {e}"))
            })?))
        }
        _ => None,
    };
    Ok(Job {
        id,
        dut: DutId(dut),
        dut_kind,
        kind,
        campaign,
        state,
        created_at,
        updated_at,
    })
}

fn row_to_campaign(row: &sqlx::sqlite::SqliteRow) -> Result<Campaign> {
    let id_str: String = row.try_get("id").map_err(DaemonError::Sqlx)?;
    let dut: String = row.try_get("dut").map_err(DaemonError::Sqlx)?;
    let chip_serial: Option<String> = row.try_get("chip_serial").map_err(DaemonError::Sqlx)?;
    let template_s: String = row.try_get("template").map_err(DaemonError::Sqlx)?;
    let state_s: String = row.try_get("state").map_err(DaemonError::Sqlx)?;
    let created_at: String = row.try_get("created_at").map_err(DaemonError::Sqlx)?;
    let updated_at: String = row.try_get("updated_at").map_err(DaemonError::Sqlx)?;

    let id = CampaignId(
        uuid::Uuid::parse_str(&id_str)
            .map_err(|e| DaemonError::Config(format!("invalid campaign id `{id_str}`: {e}")))?,
    );
    let template: CampaignTemplate = serde_json::from_str(&template_s)?;
    let state: CampaignState = serde_json::from_str(&state_s)?;
    let created_at = chrono::DateTime::parse_from_rfc3339(&created_at)
        .map_err(|e| DaemonError::Config(format!("created_at: {e}")))?
        .with_timezone(&chrono::Utc);
    let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at)
        .map_err(|e| DaemonError::Config(format!("updated_at: {e}")))?
        .with_timezone(&chrono::Utc);
    Ok(Campaign {
        id,
        dut: heimdall_core::DutId(dut),
        chip_serial,
        template,
        state,
        created_at,
        updated_at,
    })
}

#[async_trait]
impl JobStore for SqliteJobStore {
    async fn create_job(&self, new: NewJob) -> Result<Job> {
        // `dut_kind` isn't on `NewJob` yet. Encode a placeholder; the HTTP
        // layer overrides it once the DUT registry has resolved the kind.
        let dut_kind = DutKind::RiverRc1Nano;

        let now = Utc::now();
        let job = Job {
            id: JobId::new(),
            dut: new.dut.clone(),
            dut_kind,
            kind: new.kind.clone(),
            campaign: new.campaign,
            state: JobState::Queued,
            created_at: now,
            updated_at: now,
        };
        sqlx::query(
            r#"
            INSERT INTO jobs (id, dut, dut_kind, kind_json, state_json, state_tag, created_at, updated_at, campaign_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(job.id.0.to_string())
        .bind(&job.dut.0)
        .bind(dut_kind_str(&job.dut_kind))
        .bind(serde_json::to_string(&job.kind)?)
        .bind(serde_json::to_string(&job.state)?)
        .bind(serde_json::to_string(&job.state.tag())?.trim_matches('"').to_string())
        .bind(job.created_at.to_rfc3339())
        .bind(job.updated_at.to_rfc3339())
        .bind(new.campaign.as_ref().map(|c| c.0.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(job)
    }

    async fn get_job(&self, id: JobId) -> Result<Option<Job>> {
        let row = sqlx::query("SELECT * FROM jobs WHERE id = ?1")
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(row_to_job(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_jobs(&self, filter: JobFilter) -> Result<Vec<Job>> {
        let mut sql = String::from("SELECT * FROM jobs WHERE 1=1");
        if filter.dut.is_some() {
            sql.push_str(" AND dut = ?");
        }
        if let Some(tags) = &filter.state_in {
            if !tags.is_empty() {
                sql.push_str(" AND state_tag IN (");
                for (i, _) in tags.iter().enumerate() {
                    if i > 0 {
                        sql.push(',');
                    }
                    sql.push('?');
                }
                sql.push(')');
            }
        }
        sql.push_str(" ORDER BY created_at DESC");
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
        }
        let mut q = sqlx::query(&sql);
        if let Some(dut) = filter.dut.as_ref() {
            q = q.bind(&dut.0);
        }
        if let Some(tags) = filter.state_in.as_ref() {
            for tag in tags {
                let s = serde_json::to_string(tag)?.trim_matches('"').to_string();
                q = q.bind(s);
            }
        }
        let rows = q.fetch_all(&self.pool).await?;
        rows.iter().map(row_to_job).collect()
    }

    async fn update_state(&self, id: JobId, state: JobState) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
            UPDATE jobs
            SET state_json = ?1,
                state_tag = ?2,
                updated_at = ?3
            WHERE id = ?4
            "#,
        )
        .bind(serde_json::to_string(&state)?)
        .bind(
            serde_json::to_string(&state.tag())?
                .trim_matches('"')
                .to_string(),
        )
        .bind(now.to_rfc3339())
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn append_event(&self, ev: Event) -> Result<EventId> {
        let now = Utc::now();
        let row = sqlx::query(
            r#"
            INSERT INTO events (payload_json, created_at)
            VALUES (?1, ?2)
            RETURNING id
            "#,
        )
        .bind(serde_json::to_string(&ev)?)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        let id: i64 = row.try_get("id").map_err(DaemonError::Sqlx)?;
        Ok(EventId(id as u64))
    }

    async fn list_events_since(&self, since: EventId, limit: u32) -> Result<Vec<(EventId, Event)>> {
        let rows = sqlx::query(
            r#"
            SELECT id, payload_json
            FROM events
            WHERE id > ?1
            ORDER BY id ASC
            LIMIT ?2
            "#,
        )
        .bind(since.0 as i64)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: i64 = r.try_get("id").map_err(DaemonError::Sqlx)?;
            let payload: String = r.try_get("payload_json").map_err(DaemonError::Sqlx)?;
            let ev: Event = serde_json::from_str(&payload)?;
            out.push((EventId(id as u64), ev));
        }
        Ok(out)
    }

    async fn create_campaign(&self, campaign: Campaign) -> Result<Campaign> {
        sqlx::query(
            r#"
            INSERT INTO campaigns (id, dut, chip_serial, template, state, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(campaign.id.0.to_string())
        .bind(&campaign.dut.0)
        .bind(campaign.chip_serial.as_deref())
        .bind(serde_json::to_string(&campaign.template)?)
        .bind(serde_json::to_string(&campaign.state)?)
        .bind(campaign.created_at.to_rfc3339())
        .bind(campaign.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(campaign)
    }

    async fn get_campaign(&self, id: CampaignId) -> Result<Option<Campaign>> {
        let row = sqlx::query("SELECT * FROM campaigns WHERE id = ?1")
            .bind(id.0.to_string())
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(row_to_campaign(&r)?)),
            None => Ok(None),
        }
    }

    async fn list_campaigns(&self, limit: Option<u32>) -> Result<Vec<Campaign>> {
        let mut sql = String::from("SELECT * FROM campaigns ORDER BY created_at DESC");
        if let Some(l) = limit {
            sql.push_str(&format!(" LIMIT {l}"));
        }
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await?;
        rows.iter().map(row_to_campaign).collect()
    }

    async fn update_campaign_state(&self, id: CampaignId, state: CampaignState) -> Result<()> {
        let now = chrono::Utc::now();
        sqlx::query(
            r#"
            UPDATE campaigns SET state = ?1, updated_at = ?2 WHERE id = ?3
            "#,
        )
        .bind(serde_json::to_string(&state)?)
        .bind(now.to_rfc3339())
        .bind(id.0.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn list_jobs_for_campaign(&self, id: CampaignId) -> Result<Vec<Job>> {
        let rows = sqlx::query("SELECT * FROM jobs WHERE campaign_id = ?1 ORDER BY created_at ASC")
            .bind(id.0.to_string())
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_job).collect()
    }

    async fn import_job(&self, job: Job) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO jobs (id, dut, dut_kind, kind_json, state_json, state_tag, created_at, updated_at, campaign_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
        )
        .bind(job.id.0.to_string())
        .bind(&job.dut.0)
        .bind(dut_kind_str(&job.dut_kind))
        .bind(serde_json::to_string(&job.kind)?)
        .bind(serde_json::to_string(&job.state)?)
        .bind(
            serde_json::to_string(&job.state.tag())?
                .trim_matches('"')
                .to_string(),
        )
        .bind(job.created_at.to_rfc3339())
        .bind(job.updated_at.to_rfc3339())
        .bind(job.campaign.as_ref().map(|c| c.0.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn import_campaign(&self, campaign: Campaign) -> Result<()> {
        // Identical to create_campaign for sqlite: the table already takes
        // the full row including id, so re-inserting with the original id
        // preserves cross-references from jobs.campaign_id.
        sqlx::query(
            r#"
            INSERT INTO campaigns (id, dut, chip_serial, template, state, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
        )
        .bind(campaign.id.0.to_string())
        .bind(&campaign.dut.0)
        .bind(campaign.chip_serial.as_deref())
        .bind(serde_json::to_string(&campaign.template)?)
        .bind(serde_json::to_string(&campaign.state)?)
        .bind(campaign.created_at.to_rfc3339())
        .bind(campaign.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn import_event(&self, id: EventId, ev: Event) -> Result<()> {
        let now = Utc::now();
        sqlx::query(
            r#"
            INSERT INTO events (id, payload_json, created_at)
            VALUES (?1, ?2, ?3)
            "#,
        )
        .bind(id.0 as i64)
        .bind(serde_json::to_string(&ev)?)
        .bind(now.to_rfc3339())
        .execute(&self.pool)
        .await?;
        // SQLite AUTOINCREMENT respects the highest inserted id, so future
        // append_event calls will get id > max(imported_id) automatically.
        Ok(())
    }
}
