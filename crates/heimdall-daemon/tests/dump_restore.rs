//! Round-trip test for the dump/restore snapshot format.

#![cfg(feature = "sqlite")]

use chrono::Utc;
use heimdall_core::DutId;
use heimdall_daemon::{
    BlobStore, Campaign, CampaignId, CampaignState, CampaignTemplate, Event, EventId, JobFilter,
    JobKind, JobStore, LocalFsBlobStore, NewJob, SqliteJobStore, dump as snapshot,
};
use tempfile::TempDir;

async fn fresh_stores() -> (SqliteJobStore, LocalFsBlobStore, TempDir) {
    let tmp = TempDir::new().expect("tmp");
    let store = SqliteJobStore::open_in_memory().await.expect("store");
    let blobs = LocalFsBlobStore::open(tmp.path().to_path_buf())
        .await
        .expect("blobs");
    (store, blobs, tmp)
}

#[tokio::test]
async fn round_trip_preserves_jobs_campaigns_blobs() {
    let (src_store, src_blobs, _src_tmp) = fresh_stores().await;

    // Seed the source: 1 campaign, 2 jobs (one in the campaign), 2 blobs, 1 event.
    let camp = Campaign {
        id: CampaignId::new(),
        dut: DutId::new("dut-a"),
        chip_serial: Some("SN0001".into()),
        template: CampaignTemplate::BringUp,
        state: CampaignState::Pending,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    src_store.create_campaign(camp.clone()).await.unwrap();

    let j1 = src_store
        .create_job(NewJob {
            dut: DutId::new("dut-a"),
            kind: JobKind::MockHello,
            campaign: Some(camp.id),
        })
        .await
        .unwrap();
    let j2 = src_store
        .create_job(NewJob {
            dut: DutId::new("dut-b"),
            kind: JobKind::MockHello,
            campaign: None,
        })
        .await
        .unwrap();

    let b1 = src_blobs.put(b"hello world").await.unwrap();
    let b2 = src_blobs.put(b"another blob").await.unwrap();

    let ev_id = src_store
        .append_event(Event::JobCreated {
            job: j1.id,
            dut: j1.dut.clone(),
        })
        .await
        .unwrap();

    // Dump to a temp file (writer must be 'static; the file path is reusable).
    let snap_dir = TempDir::new().unwrap();
    let snap_path = snap_dir.path().join("snapshot.tar");
    let writer = std::fs::File::create(&snap_path).unwrap();
    let manifest = snapshot::dump(&src_store, &src_blobs, writer)
        .await
        .expect("dump");
    assert_eq!(manifest.job_count, 2);
    assert_eq!(manifest.campaign_count, 1);
    assert_eq!(manifest.event_count, 1);
    assert_eq!(manifest.blob_count, 2);
    assert!(snap_path.exists());

    // Restore into fresh stores.
    let (dst_store, dst_blobs, _dst_tmp) = fresh_stores().await;
    let reader = std::fs::File::open(&snap_path).unwrap();
    let stats = snapshot::restore(&dst_store, &dst_blobs, reader)
        .await
        .expect("restore");
    assert_eq!(stats.jobs_restored, 2);
    assert_eq!(stats.campaigns_restored, 1);
    assert_eq!(stats.events_restored, 1);
    assert_eq!(stats.blobs_restored, 2);

    // Verify IDs preserved.
    let restored_camp = dst_store
        .get_campaign(camp.id)
        .await
        .unwrap()
        .expect("camp");
    assert_eq!(restored_camp.id, camp.id);
    assert_eq!(restored_camp.dut, camp.dut);

    let restored_j1 = dst_store.get_job(j1.id).await.unwrap().expect("j1");
    assert_eq!(restored_j1.dut, j1.dut);
    assert_eq!(restored_j1.campaign, Some(camp.id));

    let restored_j2 = dst_store.get_job(j2.id).await.unwrap().expect("j2");
    assert_eq!(restored_j2.dut, j2.dut);
    assert!(restored_j2.campaign.is_none());

    // Blob bytes round-trip and IDs match (content-addressed).
    let restored_b1 = dst_blobs.get(&b1).await.unwrap().expect("b1 present");
    assert_eq!(&restored_b1[..], b"hello world");
    let restored_b2 = dst_blobs.get(&b2).await.unwrap().expect("b2 present");
    assert_eq!(&restored_b2[..], b"another blob");

    // Event preserved with original id.
    let evs = dst_store.list_events_since(EventId(0), 100).await.unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].0, ev_id);

    // After restore, the destination store's next append_event must produce
    // an EventId strictly greater than every restored one.
    let next = dst_store
        .append_event(Event::JobCreated {
            job: j2.id,
            dut: j2.dut.clone(),
        })
        .await
        .unwrap();
    assert!(
        next.0 > ev_id.0,
        "next event id ({}) must exceed restored ({})",
        next.0,
        ev_id.0
    );
}

#[tokio::test]
async fn restore_rejects_unknown_format_version() {
    let (dst_store, dst_blobs, _tmp) = fresh_stores().await;

    // Build a tar with a manifest that has a bogus format_version.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("bad.tar");
    {
        let f = std::fs::File::create(&path).unwrap();
        let mut tar = tar::Builder::new(f);
        let manifest = serde_json::json!({
            "format_version": 9999,
            "heimdall_version": "0.0.0",
            "exported_at": "2026-01-01T00:00:00Z",
            "job_count": 0,
            "campaign_count": 0,
            "event_count": 0,
            "blob_count": 0,
        });
        let body = serde_json::to_vec(&manifest).unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", body.as_slice())
            .unwrap();
        tar.finish().unwrap();
    }

    let result =
        snapshot::restore(&dst_store, &dst_blobs, std::fs::File::open(&path).unwrap()).await;
    assert!(result.is_err(), "expected error for unknown format_version");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains("9999") || msg.contains("format_version"),
        "got: {msg}"
    );
}

#[tokio::test]
async fn restore_rejects_missing_manifest() {
    let (dst_store, dst_blobs, _tmp) = fresh_stores().await;

    // Tar with only a stray entry, no manifest.json.
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("nomanifest.tar");
    {
        let f = std::fs::File::create(&path).unwrap();
        let mut tar = tar::Builder::new(f);
        let mut header = tar::Header::new_gnu();
        header.set_size(3);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "stray.txt", &b"abc"[..])
            .unwrap();
        tar.finish().unwrap();
    }

    let result =
        snapshot::restore(&dst_store, &dst_blobs, std::fs::File::open(&path).unwrap()).await;
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("manifest"), "got: {msg}");
}

#[tokio::test]
async fn dump_of_empty_store_is_well_formed() {
    let (store, blobs, _tmp) = fresh_stores().await;
    let snap_dir = TempDir::new().unwrap();
    let snap_path = snap_dir.path().join("empty.tar");
    let writer = std::fs::File::create(&snap_path).unwrap();
    let manifest = snapshot::dump(&store, &blobs, writer).await.unwrap();
    assert_eq!(manifest.job_count, 0);
    assert_eq!(manifest.campaign_count, 0);
    assert_eq!(manifest.event_count, 0);
    assert_eq!(manifest.blob_count, 0);
    let size = std::fs::metadata(&snap_path).unwrap().len();
    assert!(size > 0, "tar should still contain manifest");

    // And the empty snapshot restores cleanly into a fresh pair of stores.
    let (dst_store, dst_blobs, _dst_tmp) = fresh_stores().await;
    let reader = std::fs::File::open(&snap_path).unwrap();
    let stats = snapshot::restore(&dst_store, &dst_blobs, reader)
        .await
        .unwrap();
    assert_eq!(stats.jobs_restored, 0);
    assert_eq!(stats.campaigns_restored, 0);
    assert_eq!(stats.events_restored, 0);
    assert_eq!(stats.blobs_restored, 0);
    assert!(
        dst_store
            .list_jobs(JobFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
}
