//! Integration tests for SqliteJobStore against an in-memory sqlite db.

#![cfg(feature = "sqlite")]

use heimdall_core::{DutId, DutKind};
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
        .create_job(
            NewJob {
                dut: DutId::new("d1"),
                kind: JobKind::MockHello,
                campaign: None,
            },
            DutKind::RiverRc1Small,
        )
        .await
        .unwrap();
    let back = store.get_job(job.id).await.unwrap().expect("present");
    assert_eq!(back.id, job.id);
    assert_eq!(back.dut, job.dut);
    assert_eq!(back.dut_kind, DutKind::RiverRc1Small);
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
        .create_job(
            NewJob {
                dut: DutId::new("d1"),
                kind: JobKind::MockHello,
                campaign: None,
            },
            DutKind::RiverRc1Nano,
        )
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
        .create_job(
            NewJob {
                dut: DutId::new("d1"),
                kind: JobKind::MockHello,
                campaign: None,
            },
            DutKind::RiverRc1Nano,
        )
        .await
        .unwrap();
    let _b = store
        .create_job(
            NewJob {
                dut: DutId::new("d2"),
                kind: JobKind::MockHello,
                campaign: None,
            },
            DutKind::RiverRc1Nano,
        )
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
async fn list_job_logs_filters_by_kind_and_job_and_since() {
    let store = store().await;
    let job_a = heimdall_daemon::JobId::new();
    let job_b = heimdall_daemon::JobId::new();

    // Mix of JobLog rows for two different jobs and a non-log event.
    let id1 = store
        .append_event(Event::JobLog {
            job: job_a,
            level: heimdall_daemon::LogLevel::Info,
            message: "a1".into(),
            stage: Some("prepare".into()),
            i18n_key: None,
            i18n_args: Default::default(),
        })
        .await
        .unwrap();
    let _ = store
        .append_event(Event::JobLog {
            job: job_b,
            level: heimdall_daemon::LogLevel::Info,
            message: "b1".into(),
            stage: None,
            i18n_key: None,
            i18n_args: Default::default(),
        })
        .await
        .unwrap();
    let id3 = store
        .append_event(Event::JobLog {
            job: job_a,
            level: heimdall_daemon::LogLevel::Warn,
            message: "a2".into(),
            stage: Some("load".into()),
            i18n_key: None,
            i18n_args: Default::default(),
        })
        .await
        .unwrap();
    let _ = store
        .append_event(Event::JobCreated {
            job: job_a,
            dut: DutId::new("d1"),
        })
        .await
        .unwrap();

    let all_a = store
        .list_job_logs(job_a, heimdall_daemon::EventId(0), 10)
        .await
        .unwrap();
    assert_eq!(all_a.len(), 2);
    let msgs: Vec<_> = all_a
        .iter()
        .map(|rec| match &rec.event {
            Event::JobLog { message, .. } => message.clone(),
            _ => "<not-log>".into(),
        })
        .collect();
    assert_eq!(msgs, vec!["a1".to_string(), "a2".to_string()]);
    // Each record carries a stamped UTC timestamp.
    for rec in &all_a {
        assert!(rec.ts.timestamp() > 0, "ts must be populated: {rec:?}");
    }

    // since= filters to events strictly newer than id1.
    let delta = store.list_job_logs(job_a, id1, 10).await.unwrap();
    assert_eq!(delta.len(), 1);
    assert_eq!(delta[0].id.0, id3.0);
    match &delta[0].event {
        Event::JobLog { message, .. } => assert_eq!(message, "a2"),
        other => panic!("unexpected: {other:?}"),
    }
}

#[tokio::test]
async fn append_and_list_events() {
    let store = store().await;
    let id1 = store
        .append_event(Event::JobLog {
            job: heimdall_daemon::JobId::new(),
            level: heimdall_daemon::LogLevel::Info,
            message: "hello".into(),
            stage: None,
            i18n_key: None,
            i18n_args: Default::default(),
        })
        .await
        .unwrap();
    let id2 = store
        .append_event(Event::JobLog {
            job: heimdall_daemon::JobId::new(),
            level: heimdall_daemon::LogLevel::Info,
            message: "world".into(),
            stage: None,
            i18n_key: None,
            i18n_args: Default::default(),
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
        .create_job(
            NewJob {
                dut: DutId::new("d1"),
                kind: JobKind::MockHello,
                campaign: Some(campaign_id),
            },
            DutKind::RiverRc1Nano,
        )
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
async fn job_program_round_trips_via_sqlite() {
    use heimdall_core::ArtifactKind;
    use heimdall_daemon::{BlobId, JobProgramRef};

    let store = store().await;
    let job = store
        .create_job(
            NewJob {
                dut: DutId::new("river-1"),
                kind: JobKind::MockHello,
                campaign: None,
            },
            DutKind::RiverRc1Nano,
        )
        .await
        .unwrap();
    // Nothing recorded yet.
    assert!(store.get_job_program(job.id).await.unwrap().is_none());

    let r = JobProgramRef {
        blob_id: BlobId("deadbeef".into()),
        kind: ArtifactKind::RawBytes,
        iter: Some(7),
    };
    store.set_job_program(job.id, r.clone()).await.unwrap();
    let got = store
        .get_job_program(job.id)
        .await
        .unwrap()
        .expect("present");
    assert_eq!(got.blob_id.0, r.blob_id.0);
    assert!(matches!(got.kind, ArtifactKind::RawBytes));
    assert_eq!(got.iter, Some(7));

    // Upsert: subsequent writes overwrite. Pin that the table holds
    // exactly the latest, not a history.
    let r2 = JobProgramRef {
        blob_id: BlobId("c0ffee".into()),
        kind: ArtifactKind::ElfRiscv,
        iter: Some(42),
    };
    store.set_job_program(job.id, r2.clone()).await.unwrap();
    let got2 = store.get_job_program(job.id).await.unwrap().unwrap();
    assert_eq!(got2.blob_id.0, "c0ffee");
    assert!(matches!(got2.kind, ArtifactKind::ElfRiscv));
    assert_eq!(got2.iter, Some(42));
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
