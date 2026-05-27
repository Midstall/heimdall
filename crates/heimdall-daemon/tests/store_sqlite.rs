//! Integration tests for SqliteJobStore against an in-memory sqlite db.

#![cfg(feature = "sqlite")]

use heimdall_core::DutId;
use heimdall_daemon::{
    Campaign, CampaignId, CampaignState, CampaignTemplate, Event, JobFilter, JobKind, JobState,
    JobStateTag, JobStore, NewJob, SqliteJobStore, VerdictSummary,
};

async fn store() -> SqliteJobStore {
    SqliteJobStore::open_in_memory().await.expect("open")
}

#[tokio::test]
async fn create_get_roundtrip() {
    let store = store().await;
    let job = store
        .create_job(NewJob {
            dut: DutId::new("d1"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .await
        .unwrap();
    let back = store.get_job(job.id).await.unwrap().expect("present");
    assert_eq!(back.id, job.id);
    assert_eq!(back.dut, job.dut);
    assert!(matches!(back.state, JobState::Queued));
    assert!(matches!(back.kind, JobKind::MockHello));
}

#[tokio::test]
async fn get_missing_returns_none() {
    let store = store().await;
    let none = store
        .get_job(heimdall_daemon::JobId(uuid::Uuid::nil()))
        .await
        .unwrap();
    assert!(none.is_none());
}

#[tokio::test]
async fn update_state_transitions() {
    let store = store().await;
    let job = store
        .create_job(NewJob {
            dut: DutId::new("d1"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .await
        .unwrap();
    store.update_state(job.id, JobState::Running).await.unwrap();
    let after = store.get_job(job.id).await.unwrap().unwrap();
    assert!(matches!(after.state, JobState::Running));

    let done = JobState::Done(VerdictSummary::Pass);
    store.update_state(job.id, done).await.unwrap();
    let after = store.get_job(job.id).await.unwrap().unwrap();
    assert!(matches!(after.state, JobState::Done(VerdictSummary::Pass)));
}

#[tokio::test]
async fn list_filters_by_state() {
    let store = store().await;
    let a = store
        .create_job(NewJob {
            dut: DutId::new("d1"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .await
        .unwrap();
    let _b = store
        .create_job(NewJob {
            dut: DutId::new("d2"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .await
        .unwrap();
    store
        .update_state(a.id, JobState::Done(VerdictSummary::Pass))
        .await
        .unwrap();

    let queued = store
        .list_jobs(JobFilter {
            state_in: Some(vec![JobStateTag::Queued]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(queued.len(), 1);
    let done = store
        .list_jobs(JobFilter {
            state_in: Some(vec![JobStateTag::Done]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].id, a.id);
}

#[tokio::test]
async fn append_and_list_events() {
    let store = store().await;
    let id1 = store
        .append_event(Event::JobLog {
            job: heimdall_daemon::JobId::new(),
            level: "info".into(),
            message: "hello".into(),
        })
        .await
        .unwrap();
    let id2 = store
        .append_event(Event::JobLog {
            job: heimdall_daemon::JobId::new(),
            level: "info".into(),
            message: "world".into(),
        })
        .await
        .unwrap();
    assert!(id2.0 > id1.0);
    let events = store
        .list_events_since(heimdall_daemon::EventId(0), 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 2);
}

#[tokio::test]
async fn campaign_create_get_roundtrip() {
    let store = store().await;
    let now = chrono::Utc::now();
    let campaign = Campaign {
        id: CampaignId::new(),
        dut: heimdall_core::DutId::new("d1"),
        chip_serial: Some("R1N-0001".into()),
        template: CampaignTemplate::BringUp,
        state: CampaignState::Pending,
        created_at: now,
        updated_at: now,
    };
    let id = campaign.id;
    store.create_campaign(campaign).await.unwrap();
    let back = store.get_campaign(id).await.unwrap().expect("present");
    assert_eq!(back.id, id);
    assert!(matches!(back.template, CampaignTemplate::BringUp));
    assert!(matches!(back.state, CampaignState::Pending));
}

#[tokio::test]
async fn job_with_campaign_id_roundtrips() {
    let store = store().await;
    let campaign_id = CampaignId::new();
    let now = chrono::Utc::now();
    let campaign = Campaign {
        id: campaign_id,
        dut: heimdall_core::DutId::new("d1"),
        chip_serial: None,
        template: CampaignTemplate::BringUp,
        state: CampaignState::Pending,
        created_at: now,
        updated_at: now,
    };
    store.create_campaign(campaign).await.unwrap();

    let job = store
        .create_job(NewJob {
            dut: DutId::new("d1"),
            kind: JobKind::MockHello,
            campaign: Some(campaign_id),
        })
        .await
        .unwrap();
    assert_eq!(job.campaign, Some(campaign_id));

    let back = store.get_job(job.id).await.unwrap().expect("present");
    assert_eq!(back.campaign, Some(campaign_id));

    let jobs = store.list_jobs_for_campaign(campaign_id).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job.id);
}

#[tokio::test]
async fn update_campaign_state() {
    let store = store().await;
    let now = chrono::Utc::now();
    let id = CampaignId::new();
    store
        .create_campaign(Campaign {
            id,
            dut: heimdall_core::DutId::new("d1"),
            chip_serial: None,
            template: CampaignTemplate::BringUp,
            state: CampaignState::Pending,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    store
        .update_campaign_state(id, CampaignState::Running)
        .await
        .unwrap();
    let after = store.get_campaign(id).await.unwrap().unwrap();
    assert!(matches!(after.state, CampaignState::Running));
}
