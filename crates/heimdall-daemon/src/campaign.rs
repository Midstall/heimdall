//! Campaign runtime: submit a campaign (creates the Campaign row plus its
//! constituent jobs via the JobQueue) and recompute campaign state from its
//! children.

use chrono::Utc;
use heimdall_core::{DutId, DutKind};
use tracing::{info, instrument};

use crate::error::Result;
use crate::queue::JobQueue;
use crate::templates;
use crate::types::{
    Campaign, CampaignId, CampaignState, CampaignTemplate, Event, Job, JobState, JobStateTag,
    VerdictSummary,
};

/// Create a campaign with the given template, submit each constituent job
/// through the queue, and publish a `CampaignCreated` event. Returns the
/// freshly-persisted Campaign.
#[instrument(skip(queue, bringup), fields(template = template.name(), dut = %dut.0))]
pub async fn submit_campaign(
    queue: &JobQueue,
    template: CampaignTemplate,
    dut: DutId,
    dut_kind: DutKind,
    chip_serial: Option<String>,
    bringup: Option<&crate::dut_registry::BringupPayload>,
) -> Result<Campaign> {
    let store = queue.store();
    let now = Utc::now();
    let campaign = Campaign {
        id: CampaignId::new(),
        dut: dut.clone(),
        chip_serial: chip_serial.clone(),
        template: template.clone(),
        state: CampaignState::Pending,
        created_at: now,
        updated_at: now,
    };
    let campaign = store.create_campaign(campaign).await?;

    queue
        .bus()
        .publish(Event::CampaignCreated {
            campaign: campaign.id,
            dut: dut.clone(),
            template: template.clone(),
        })
        .await?;

    let new_jobs = templates::build(&template, dut, dut_kind, bringup, campaign.id);
    if new_jobs.is_empty() {
        // Empty template. Flip directly to Pass (nothing to fail).
        store
            .update_campaign_state(campaign.id, CampaignState::Pass)
            .await?;
        queue
            .bus()
            .publish(Event::CampaignStateChanged {
                campaign: campaign.id,
                state: CampaignState::Pass,
            })
            .await?;
        return Ok(Campaign {
            state: CampaignState::Pass,
            ..campaign
        });
    }

    for new in new_jobs {
        queue.submit(new).await?;
    }

    // Move to Running once we've actually submitted jobs.
    store
        .update_campaign_state(campaign.id, CampaignState::Running)
        .await?;
    queue
        .bus()
        .publish(Event::CampaignStateChanged {
            campaign: campaign.id,
            state: CampaignState::Running,
        })
        .await?;

    info!(campaign = %campaign.id, "submitted");
    Ok(Campaign {
        state: CampaignState::Running,
        ..campaign
    })
}

/// Derive a CampaignState from the states of its child jobs.
/// - Any Queued or Running -> Running
/// - All terminal (Done/Failed/Cancelled): aggregate
///   - All Done(Pass) -> Pass
///   - Any Failed or Done(Fail/Error) -> Fail
///   - Mix of Pass + Skip -> Pass (skips are cosmetic only)
///   - Pass + Fail mix -> Mixed
///   - All Cancelled -> Cancelled
/// - Empty jobs slice -> Pending
pub fn compute_state(jobs: &[Job]) -> CampaignState {
    if jobs.is_empty() {
        return CampaignState::Pending;
    }
    let mut any_running = false;
    let mut any_pass = false;
    let mut any_fail = false;
    let mut all_cancelled = true;
    for j in jobs {
        match j.state.tag() {
            JobStateTag::Queued | JobStateTag::Running => {
                any_running = true;
                all_cancelled = false;
            }
            JobStateTag::Done => {
                all_cancelled = false;
                if let JobState::Done(v) = &j.state {
                    match v {
                        VerdictSummary::Pass => any_pass = true,
                        VerdictSummary::Fail { .. } | VerdictSummary::Error { .. } => {
                            any_fail = true
                        }
                        VerdictSummary::Skip { .. } => {}
                    }
                }
            }
            JobStateTag::Failed => {
                any_fail = true;
                all_cancelled = false;
            }
            JobStateTag::Cancelled => {}
        }
    }

    if any_running {
        return CampaignState::Running;
    }
    if all_cancelled {
        return CampaignState::Cancelled;
    }
    match (any_pass, any_fail) {
        (true, false) => CampaignState::Pass,
        (false, true) => CampaignState::Fail,
        (true, true) => CampaignState::Mixed,
        (false, false) => CampaignState::Pass, // all skip
    }
}

/// Recompute a campaign's state from its children and persist if changed.
/// Returns the (possibly-updated) state.
pub async fn refresh_state(queue: &JobQueue, campaign_id: CampaignId) -> Result<CampaignState> {
    let store = queue.store();
    let jobs = store.list_jobs_for_campaign(campaign_id).await?;
    let computed = compute_state(&jobs);
    let current = store.get_campaign(campaign_id).await?;
    if let Some(c) = current {
        if c.state != computed {
            store
                .update_campaign_state(campaign_id, computed.clone())
                .await?;
            queue
                .bus()
                .publish(Event::CampaignStateChanged {
                    campaign: campaign_id,
                    state: computed.clone(),
                })
                .await?;
        }
    }
    Ok(computed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{JobId, JobKind, NewJob};
    use heimdall_core::DutKind;

    fn fake_job(state: JobState) -> Job {
        let now = Utc::now();
        Job {
            id: JobId::new(),
            dut: DutId::new("d1"),
            dut_kind: DutKind::RiverRc1Nano,
            kind: JobKind::MockHello,
            campaign: None,
            state,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn empty_is_pending() {
        assert_eq!(compute_state(&[]), CampaignState::Pending);
    }

    #[test]
    fn all_pass_is_pass() {
        let jobs = vec![
            fake_job(JobState::Done(VerdictSummary::Pass)),
            fake_job(JobState::Done(VerdictSummary::Pass)),
        ];
        assert_eq!(compute_state(&jobs), CampaignState::Pass);
    }

    #[test]
    fn any_fail_is_fail() {
        let jobs = vec![
            fake_job(JobState::Done(VerdictSummary::Pass)),
            fake_job(JobState::Failed("infra".into())),
        ];
        assert_eq!(compute_state(&jobs), CampaignState::Mixed);
    }

    #[test]
    fn any_running_is_running() {
        let jobs = vec![
            fake_job(JobState::Done(VerdictSummary::Pass)),
            fake_job(JobState::Running),
        ];
        assert_eq!(compute_state(&jobs), CampaignState::Running);
    }

    #[test]
    fn all_cancelled_is_cancelled() {
        let jobs = vec![fake_job(JobState::Cancelled), fake_job(JobState::Cancelled)];
        assert_eq!(compute_state(&jobs), CampaignState::Cancelled);
    }

    // Touch NewJob to silence unused-import warnings if needed.
    #[allow(dead_code)]
    fn _t() {
        let _ = NewJob {
            dut: DutId::new("x"),
            kind: JobKind::MockHello,
            campaign: None,
        };
        let _: DutKind = DutKind::RiverRc1Nano;
    }
}
