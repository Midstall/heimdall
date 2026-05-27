use heimdall_core::{
    Artifact, ArtifactKind, DutId, DutKind, Evidence, FailureKind, RunId, SeedId, SkipReason,
    State, StepBudget, Stimulus, ValueRepr,
};

#[test]
fn dut_kind_kebab_case() {
    let s = serde_json::to_string(&DutKind::RiverRc1Medium).unwrap();
    assert_eq!(s, "\"river-rc1-medium\"");
}

#[test]
fn state_roundtrip() {
    let s = State::new()
        .with("pc", ValueRepr::U64(0x8000_0010))
        .with("a0", ValueRepr::U64(0x42));
    let j = serde_json::to_string(&s).unwrap();
    let back: State = serde_json::from_str(&j).unwrap();
    assert_eq!(back, s);
}

#[test]
fn failure_kind_serialization_is_kebab_tagged() {
    let k = FailureKind::DutUnresponsive { millis: 1500 };
    let j = serde_json::to_string(&k).unwrap();
    assert!(j.contains("\"kind\":\"dut-unresponsive\""));
}

#[test]
fn ids_construction() {
    let _ = DutId::new("luna1-die-7");
    let _ = RunId::new();
    let _ = SeedId(1);
}

#[test]
fn artifact_kind_bitstream_variant() {
    let k = ArtifactKind::Bitstream {
        format: heimdall_core::BitstreamFormat::AegisRaw,
    };
    let j = serde_json::to_string(&k).unwrap();
    assert!(j.contains("\"kind\":\"bitstream\""));
    assert!(j.contains("\"format\":\"aegis-raw\""));
}

#[test]
fn stimulus_budget_cycles() {
    // NOTE: StepBudget::Cycles was changed in A6 from newtype Cycles(u64) to
    // struct variant Cycles { count: u64 }. Use the StepBudget::cycles(N) constructor.
    let s = Stimulus::new(StepBudget::cycles(2048));
    let j = serde_json::to_string(&s).unwrap();
    let back: Stimulus = serde_json::from_str(&j).unwrap();
    assert_eq!(back, s);
}

#[test]
fn evidence_skipreason_construct() {
    let _ = Evidence {
        label: "log".into(),
        detail: "x".into(),
    };
    let _ = SkipReason::Cosmetic;
}

#[test]
fn artifact_new_assigns_sha() {
    let a = Artifact::new(ArtifactKind::RawBytes, &b"abc"[..]);
    assert_eq!(
        a.sha256(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
