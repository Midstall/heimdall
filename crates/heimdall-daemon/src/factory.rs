//! DriverFactory: pluggable construction of (TestDriver, GoldenModel,
//! Test) tuples for a given Job. Worker calls into this instead of
//! hardcoding mock drivers.

use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "aegis")]
use heimdall_core::BitstreamFormat;
use heimdall_core::{Artifact, ArtifactKind, DutKind, State, StepBudget, ValueRepr};
use heimdall_driver::{Dut, MockDriver, TestDriver};
use heimdall_golden::{GoldenModel, MockGoldenModel};
use heimdall_test::{BuildCtx, Plan, Test, TestError};
use tracing::instrument;

use crate::error::{DaemonError, Result};
use crate::types::{Job, JobKind};

/// A bundle a factory returns: ready-to-run driver, golden model, and test.
/// Boxed so the worker can hold them as dyn trait objects.
pub struct DispatchBundle {
    pub driver: Box<dyn TestDriver>,
    pub golden: Box<dyn GoldenModel>,
    pub test: Box<dyn Test>,
}

impl std::fmt::Debug for DispatchBundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchBundle")
            .field("driver", &self.driver.target())
            .finish_non_exhaustive()
    }
}

/// Constructs the per-job machinery. One implementation per JobKind family.
#[async_trait]
pub trait DriverFactory: Send + Sync {
    /// Whether this factory handles the given JobKind.
    fn handles(&self, kind: &JobKind) -> bool;

    /// Build the dispatch bundle for the job.
    async fn build(&self, job: &Job) -> Result<DispatchBundle>;
}

/// Maps JobKinds to factories. The worker calls dispatch() per job and the
/// registry picks the first factory that claims the JobKind.
#[derive(Default, Clone)]
pub struct DriverRegistry {
    factories: Vec<Arc<dyn DriverFactory>>,
}

impl DriverRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with(mut self, factory: Arc<dyn DriverFactory>) -> Self {
        self.factories.push(factory);
        self
    }

    /// Default registry: MockHelloFactory, MockBootRiverElfFactory, plus (when
    /// the `aegis` feature is enabled) the AegisLoadMockFactory and
    /// AegisRealMockFactory (backed by a mock transport).
    pub fn default_mock() -> Self {
        #[cfg_attr(not(feature = "aegis"), allow(unused_mut))]
        let mut r = Self::new()
            .with(Arc::new(MockHelloFactory))
            .with(Arc::new(MockBootRiverElfFactory));
        #[cfg(feature = "aegis")]
        {
            r = r.with(Arc::new(AegisLoadMockFactory));
        }
        r
    }

    /// Default registry plus the AegisRealFactory backed by the given
    /// DutRegistry. When the `aegis` feature is off, equivalent to default_mock.
    #[cfg_attr(
        not(any(feature = "aegis", feature = "river")),
        allow(unused_variables)
    )]
    pub fn default_with_registry(dut_registry: std::sync::Arc<crate::DutRegistry>) -> Self {
        #[cfg_attr(not(any(feature = "aegis", feature = "river")), allow(unused_mut))]
        let mut r = Self::new().with(Arc::new(MockHelloFactory));
        #[cfg(feature = "aegis")]
        {
            r = r.with(Arc::new(AegisRealFactory {
                registry: dut_registry.clone(),
            }));
        }
        #[cfg(feature = "river")]
        {
            r = r.with(Arc::new(RiverRealFactory {
                registry: dut_registry.clone(),
            }));
        }
        let _ = &dut_registry; // silence unused warning if no features
        r
    }

    pub async fn dispatch(&self, job: &Job) -> Result<DispatchBundle> {
        for f in &self.factories {
            if f.handles(&job.kind) {
                return f.build(job).await;
            }
        }
        Err(DaemonError::Config(format!(
            "no factory handles JobKind {:?}",
            job.kind
        )))
    }
}

// ===== MockHelloFactory =====

/// MockDriver + MockGoldenModel running a MockHello test. Used for offline
/// development and the default `cargo run -p heimdall-test --example mock_bringup`.
pub struct MockHelloFactory;

#[async_trait]
impl DriverFactory for MockHelloFactory {
    fn handles(&self, kind: &JobKind) -> bool {
        matches!(kind, JobKind::MockHello)
    }

    #[instrument(skip(self, _job))]
    async fn build(&self, _job: &Job) -> Result<DispatchBundle> {
        let driver = MockDriver::new(DutKind::RiverRc1Nano)
            .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
        let golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
        let test = MockHelloTest {
            target: DutKind::RiverRc1Nano,
        };
        Ok(DispatchBundle {
            driver: Box::new(driver),
            golden: Box::new(golden),
            test: Box::new(test),
        })
    }
}

struct MockHelloTest {
    target: DutKind,
}

#[async_trait]
impl Test for MockHelloTest {
    fn name(&self) -> &str {
        "mock-hello"
    }
    fn target(&self) -> DutKind {
        self.target
    }
    async fn build(&self, _ctx: &mut BuildCtx<'_>) -> std::result::Result<Plan, TestError> {
        Ok(Plan {
            input: Artifact::new(ArtifactKind::Asm, &b"li a0, 0x42"[..]),
            expected: State::new().with("a0", ValueRepr::U64(0x42)),
            budget: StepBudget::cycles(1000),
            inputs: std::collections::BTreeMap::new(),
        })
    }
}

// ===== MockBootRiverElfFactory =====

/// Mock factory for BootRiverElf jobs. Used by integration tests that exercise
/// the HTTP plumbing and campaign lifecycle without real JTAG hardware.
/// Returns a MockDriver + MockGoldenModel + a trivial pass plan.
pub struct MockBootRiverElfFactory;

#[async_trait]
impl DriverFactory for MockBootRiverElfFactory {
    fn handles(&self, kind: &JobKind) -> bool {
        matches!(kind, JobKind::BootRiverElf { .. })
    }

    #[instrument(skip(self, job))]
    async fn build(&self, job: &Job) -> Result<DispatchBundle> {
        let driver =
            MockDriver::new(job.dut_kind).with_state(State::new().with("a0", ValueRepr::U64(0x42)));
        let golden = MockGoldenModel::new(job.dut_kind)
            .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
        let test = MockBootRiverElfTest {
            target: job.dut_kind,
        };
        Ok(DispatchBundle {
            driver: Box::new(driver),
            golden: Box::new(golden),
            test: Box::new(test),
        })
    }
}

struct MockBootRiverElfTest {
    target: DutKind,
}

#[async_trait]
impl Test for MockBootRiverElfTest {
    fn name(&self) -> &str {
        "mock-boot-river-elf"
    }
    fn target(&self) -> DutKind {
        self.target
    }
    async fn build(&self, _ctx: &mut BuildCtx<'_>) -> std::result::Result<Plan, TestError> {
        Ok(Plan {
            input: Artifact::new(ArtifactKind::ElfRiscv, &b"\x7fELF"[..]),
            expected: State::new().with("a0", ValueRepr::U64(0x42)),
            budget: StepBudget::cycles(1000),
            inputs: std::collections::BTreeMap::new(),
        })
    }
}

// Dut stays imported so downstream factory implementations have it in
// scope. Without this referent the lint would flag it as unused.
#[allow(dead_code)]
fn _assert_dut_importable(_: Dut) {}

// ===== AegisLoadMockFactory + AegisRealFactory =====

#[cfg(feature = "aegis")]
mod aegis_factory {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use super::*;
    use base64::Engine;
    use heimdall_driver::aegis::AegisFpgaDriver;
    use heimdall_transport::bitbang_jtag::{BitbangJtagTransport, BitbangPins};
    #[cfg(target_os = "linux")]
    use heimdall_transport::gpio_cdev::GpioCdevTransport;
    use heimdall_transport::mock::MockTransport;
    use heimdall_transport::openocd::OpenOcdJtagTransport;

    use crate::dut_registry::{DutRecord, DutRegistry, GpioSpec, TransportSpec};

    /// Tap name the Aegis OpenOCD config must declare (e.g.
    /// `jtag newtap aegis cpu -irlen 4`).
    const AEGIS_TAP_NAME: &str = "aegis.cpu";

    /// Runs `AegisFpgaDriver::load` against a MockTransport-backed bitbang
    /// JTAG. Used when there is no DutRegistry or when the registry's spec
    /// for a DUT is `Mock`. See `AegisRealFactory` for spec-driven dispatch.
    pub struct AegisLoadMockFactory;

    #[async_trait]
    impl DriverFactory for AegisLoadMockFactory {
        fn handles(&self, kind: &JobKind) -> bool {
            matches!(kind, JobKind::LoadAegisBitstream { .. })
        }

        async fn build(&self, job: &Job) -> Result<DispatchBundle> {
            let (descriptor_json, bitstream) = decode_load_bitstream(&job.kind)?;
            let mock = MockTransport::new();
            let pins = BitbangPins {
                tck: 0,
                tms: 1,
                tdi: 2,
                tdo: 3,
            };
            let jtag = BitbangJtagTransport::new(mock, pins)
                .with_clock_delay(std::time::Duration::from_nanos(1));
            let driver = AegisFpgaDriver::new(job.dut_kind, jtag);
            let golden = MockGoldenModel::new(job.dut_kind).with_state(State::new());
            Ok(DispatchBundle {
                driver: Box::new(driver),
                golden: Box::new(golden),
                test: Box::new(AegisLoadTest {
                    target: job.dut_kind,
                    descriptor_json,
                    bitstream,
                }),
            })
        }
    }

    /// Real factory that branches on the DUT's TransportSpec from the
    /// configured DutRegistry. Falls back to Mock when the DUT is not in the
    /// registry (so the daemon still works when no heimdall.toml is loaded).
    /// Handles both `LoadAegisBitstream` and `RunAegisVector` job kinds.
    pub struct AegisRealFactory {
        pub registry: Arc<DutRegistry>,
    }

    #[async_trait]
    impl DriverFactory for AegisRealFactory {
        fn handles(&self, kind: &JobKind) -> bool {
            matches!(
                kind,
                JobKind::LoadAegisBitstream { .. } | JobKind::RunAegisVector { .. }
            )
        }

        async fn build(&self, job: &Job) -> Result<DispatchBundle> {
            let golden = Box::new(MockGoldenModel::new(job.dut_kind).with_state(State::new()))
                as Box<dyn heimdall_golden::GoldenModel>;

            let dut_record = self.registry.lookup(&job.dut).cloned();
            let spec = dut_record
                .as_ref()
                .map(|r| r.jtag.clone())
                .unwrap_or(TransportSpec::Mock);

            // Build the test object from the JobKind.
            let test: Box<dyn Test> = match &job.kind {
                JobKind::LoadAegisBitstream {
                    descriptor_json,
                    bitstream_b64,
                } => {
                    let bitstream = decode_b64(bitstream_b64)?;
                    Box::new(AegisLoadTest {
                        target: job.dut_kind,
                        descriptor_json: descriptor_json.clone(),
                        bitstream,
                    })
                }
                JobKind::RunAegisVector {
                    descriptor_json,
                    bitstream_b64,
                    inputs,
                    expected_outputs,
                    settle_cycles,
                } => {
                    let bitstream = decode_b64(bitstream_b64)?;
                    Box::new(AegisVectorTest {
                        target: job.dut_kind,
                        descriptor_json: descriptor_json.clone(),
                        bitstream,
                        inputs: inputs.clone(),
                        expected_outputs: expected_outputs.clone(),
                        settle_cycles: *settle_cycles,
                    })
                }
                _ => {
                    return Err(crate::error::DaemonError::Config(
                        "aegis factory called with wrong JobKind".into(),
                    ));
                }
            };

            // Build the driver, attaching a pinmap if the DUT record has one.
            let driver: Box<dyn TestDriver> = match spec {
                TransportSpec::Mock => build_mock_driver(job.dut_kind, dut_record.as_ref()),
                #[cfg(target_os = "linux")]
                TransportSpec::BitbangCdev {
                    device,
                    tck,
                    tms,
                    tdi,
                    tdo,
                    freq_hz,
                } => {
                    let gpio = GpioCdevTransport::new(device);
                    let half = if freq_hz == 0 {
                        std::time::Duration::from_micros(5)
                    } else {
                        std::time::Duration::from_nanos((1_000_000_000u64 / freq_hz as u64) / 2)
                    };
                    let pins = BitbangPins { tck, tms, tdi, tdo };
                    let jtag = BitbangJtagTransport::new(gpio, pins).with_clock_delay(half);
                    let mut d = AegisFpgaDriver::new(job.dut_kind, jtag);
                    attach_pinmap(&mut d, dut_record.as_ref());
                    Box::new(d)
                }
                #[cfg(not(target_os = "linux"))]
                TransportSpec::BitbangCdev { .. } => {
                    return Err(crate::error::DaemonError::Config(
                        "BitbangCdev transport requires Linux (/dev/gpiochip*)".into(),
                    ));
                }
                TransportSpec::Openocd { endpoint } => {
                    let ocd = OpenOcdJtagTransport::new(endpoint).with_tap_name(AEGIS_TAP_NAME);
                    let mut d = AegisFpgaDriver::new(job.dut_kind, ocd);
                    attach_pinmap(&mut d, dut_record.as_ref());
                    Box::new(d)
                }
                TransportSpec::OpenocdSpawned {
                    binary,
                    config_file,
                    tcl_port,
                    extra_args,
                } => {
                    let spawned =
                        heimdall_transport::openocd::spawn::SpawnedOpenocdJtagTransport::new(
                            binary.clone(),
                            config_file.clone(),
                            tcl_port,
                        )
                        .with_extra_args(extra_args.clone())
                        .with_tap_name(AEGIS_TAP_NAME);
                    let mut d = AegisFpgaDriver::new(job.dut_kind, spawned);
                    attach_pinmap(&mut d, dut_record.as_ref());
                    Box::new(d)
                }
                TransportSpec::Ftdi { .. } => {
                    return Err(crate::error::DaemonError::Config(
                        "ftdi JTAG driver is not yet supported; use openocd or bitbang-jtag".into(),
                    ));
                }
            };

            Ok(DispatchBundle {
                driver,
                golden,
                test,
            })
        }
    }

    /// Decode a base64-encoded bitstream from a JobKind field.
    fn decode_b64(b64: &str) -> Result<Vec<u8>> {
        base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .map_err(|e| crate::error::DaemonError::Config(format!("bitstream base64 decode: {e}")))
    }

    /// Decode the descriptor_json + bitstream_b64 fields from a
    /// LoadAegisBitstream JobKind.
    fn decode_load_bitstream(kind: &JobKind) -> Result<(String, Vec<u8>)> {
        match kind {
            JobKind::LoadAegisBitstream {
                descriptor_json,
                bitstream_b64,
            } => Ok((descriptor_json.clone(), decode_b64(bitstream_b64)?)),
            _ => Err(crate::error::DaemonError::Config(
                "factory called with wrong JobKind".into(),
            )),
        }
    }

    /// Build a mock-transport-backed driver and attach a pinmap if available.
    fn build_mock_driver(target: DutKind, dut_record: Option<&DutRecord>) -> Box<dyn TestDriver> {
        let mock = MockTransport::new();
        let pins = BitbangPins {
            tck: 0,
            tms: 1,
            tdi: 2,
            tdo: 3,
        };
        let jtag = BitbangJtagTransport::new(mock, pins)
            .with_clock_delay(std::time::Duration::from_nanos(1));
        let mut d = AegisFpgaDriver::new(target, jtag);
        attach_pinmap(&mut d, dut_record);
        Box::new(d)
    }

    /// Attach pinmap + GPIO transport to a driver when the DUT record supplies
    /// one. Converts daemon-side `PadEntry` to driver-side entries.
    fn attach_pinmap<T>(driver: &mut AegisFpgaDriver<T>, dut_record: Option<&DutRecord>)
    where
        T: heimdall_transport::Transport + heimdall_transport::JtagOps + Send + Sync,
    {
        let Some(record) = dut_record else { return };
        if record.pad_map.entries.is_empty() {
            return;
        }

        let mut map = heimdall_driver::aegis::pinmap::IoPinmap::new();
        for e in &record.pad_map.entries {
            map = map.with(heimdall_driver::aegis::pinmap::PadEntry {
                direction: match e.direction {
                    crate::dut_registry::PadDirection::In => {
                        heimdall_driver::aegis::pinmap::PadDirection::In
                    }
                    crate::dut_registry::PadDirection::Out => {
                        heimdall_driver::aegis::pinmap::PadDirection::Out
                    }
                },
                fpga_pad: e.fpga_pad,
                gpio_line: e.gpio_line,
            });
        }

        let gpio: Box<dyn heimdall_transport::GpioTransport> = match &record.pad_map.gpio_spec {
            #[cfg(target_os = "linux")]
            Some(GpioSpec::LinuxCdev { device }) => {
                Box::new(GpioCdevTransport::new(device.clone()))
            }
            #[cfg(not(target_os = "linux"))]
            Some(GpioSpec::LinuxCdev { .. }) => Box::new(MockTransport::new()),
            Some(GpioSpec::Mock) | None => Box::new(MockTransport::new()),
        };

        driver.pad_map = map;
        driver.gpio = Some(gpio);
    }

    pub struct AegisLoadTest {
        target: DutKind,
        descriptor_json: String,
        bitstream: Vec<u8>,
    }

    #[async_trait]
    impl Test for AegisLoadTest {
        fn name(&self) -> &str {
            "aegis-load"
        }
        fn target(&self) -> DutKind {
            self.target
        }
        async fn build(&self, _ctx: &mut BuildCtx<'_>) -> std::result::Result<Plan, TestError> {
            Ok(Plan {
                input: Artifact::new(
                    ArtifactKind::Bitstream {
                        format: BitstreamFormat::AegisRaw,
                    },
                    pack_image(&self.descriptor_json, &self.bitstream)?,
                ),
                expected: State::new(),
                budget: StepBudget::cycles(1),
                inputs: BTreeMap::new(),
            })
        }
    }

    /// Test that loads an Aegis bitstream, drives input pads from the inputs
    /// map, settles for `settle_cycles`, then reads output pads and diffs
    /// against `expected_outputs`.
    pub struct AegisVectorTest {
        pub target: DutKind,
        pub descriptor_json: String,
        pub bitstream: Vec<u8>,
        pub inputs: BTreeMap<String, bool>,
        pub expected_outputs: BTreeMap<String, bool>,
        pub settle_cycles: u64,
    }

    #[async_trait]
    impl Test for AegisVectorTest {
        fn name(&self) -> &str {
            "aegis-vector"
        }
        fn target(&self) -> DutKind {
            self.target
        }
        async fn build(&self, _ctx: &mut BuildCtx<'_>) -> std::result::Result<Plan, TestError> {
            // Build the expected State from expected_outputs.
            let mut expected = State::new();
            for (k, v) in &self.expected_outputs {
                expected = expected.with(k.clone(), heimdall_core::ValueRepr::Bool(*v));
            }
            Ok(Plan {
                input: Artifact::new(
                    ArtifactKind::Bitstream {
                        format: BitstreamFormat::AegisRaw,
                    },
                    pack_image(&self.descriptor_json, &self.bitstream)?,
                ),
                expected,
                budget: StepBudget::cycles(self.settle_cycles.max(1)),
                inputs: self.inputs.clone(),
            })
        }
    }

    /// Pack (descriptor_json, bitstream) into the length-prefixed wire format
    /// expected by AegisFpgaDriver::load.
    fn pack_image(
        descriptor_json: &str,
        bitstream: &[u8],
    ) -> std::result::Result<Vec<u8>, TestError> {
        let mut packed = Vec::with_capacity(4 + descriptor_json.len() + bitstream.len());
        let len = u32::try_from(descriptor_json.len())
            .map_err(|_| TestError::Build("descriptor too large".into()))?;
        packed.extend_from_slice(&len.to_le_bytes());
        packed.extend_from_slice(descriptor_json.as_bytes());
        packed.extend_from_slice(bitstream);
        Ok(packed)
    }
}

#[cfg(feature = "aegis")]
pub use aegis_factory::{AegisLoadMockFactory, AegisRealFactory, AegisVectorTest};

// ===== RiverRealFactory =====

#[cfg(feature = "river")]
mod river_factory {
    use std::sync::Arc;

    use super::*;
    use base64::Engine;
    use heimdall_driver::river::RiverCpuDriver;
    use heimdall_golden::{MockGoldenModel, spike::SpikeOneShot};
    use heimdall_transport::openocd::OpenOcdJtagTransport;
    use heimdall_transport::openocd::spawn::SpawnedOpenocdJtagTransport;

    use crate::dut_registry::{DutRegistry, GoldenSpec, TransportSpec};

    /// Real factory for BootRiverElf jobs. Constructs RiverCpuDriver against
    /// the configured OpenOCD transport and the configured golden (Spike or
    /// Mock).
    pub struct RiverRealFactory {
        pub registry: Arc<DutRegistry>,
    }

    #[async_trait]
    impl DriverFactory for RiverRealFactory {
        fn handles(&self, kind: &JobKind) -> bool {
            matches!(kind, JobKind::BootRiverElf { .. })
        }

        async fn build(&self, job: &Job) -> Result<DispatchBundle> {
            let (elf_bytes, cycles) = decode_river_kind(&job.kind)?;

            let dut = self.registry.lookup(&job.dut).ok_or_else(|| {
                crate::error::DaemonError::Config(format!(
                    "river factory: dut `{}` not in registry",
                    job.dut.0
                ))
            })?;

            // Build the driver boxed as dyn TestDriver so both transport arms
            // can unify behind a single type-erased pointer.
            let driver: Box<dyn TestDriver> = match &dut.jtag {
                TransportSpec::Openocd { endpoint } => {
                    let jtag = OpenOcdJtagTransport::new(*endpoint);
                    Box::new(RiverCpuDriver::new(job.dut_kind, jtag))
                }
                TransportSpec::OpenocdSpawned {
                    binary,
                    config_file,
                    tcl_port,
                    extra_args,
                } => {
                    let spawned = SpawnedOpenocdJtagTransport::new(
                        binary.clone(),
                        config_file.clone(),
                        *tcl_port,
                    )
                    .with_extra_args(extra_args.clone());
                    Box::new(RiverCpuDriver::new(job.dut_kind, spawned))
                }
                other => {
                    return Err(crate::error::DaemonError::Config(format!(
                        "river factory only supports openocd or openocd-spawn transport; got {other:?}"
                    )));
                }
            };

            // Build golden.
            let golden_spec = self
                .registry
                .golden_for(job.dut_kind)
                .cloned()
                .unwrap_or(GoldenSpec::Mock);
            let golden: Box<dyn heimdall_golden::GoldenModel> = match golden_spec {
                GoldenSpec::Mock => Box::new(MockGoldenModel::new(job.dut_kind)),
                GoldenSpec::SpikeOneShot { binary, extra_args } => {
                    Box::new(SpikeOneShot::new(binary, job.dut_kind).with_extra_args(extra_args))
                }
                GoldenSpec::DartRpc { .. } => {
                    return Err(crate::error::DaemonError::Config(
                        "river factory: DartRpc golden is not yet supported".into(),
                    ));
                }
            };

            let test = Box::new(BootRiverElfTest {
                target: job.dut_kind,
                elf: elf_bytes,
                cycles,
            });

            Ok(DispatchBundle {
                driver,
                golden,
                test,
            })
        }
    }

    fn decode_river_kind(kind: &JobKind) -> Result<(Vec<u8>, u64)> {
        match kind {
            JobKind::BootRiverElf { elf_b64, cycles } => {
                let elf = base64::engine::general_purpose::STANDARD
                    .decode(elf_b64.as_bytes())
                    .map_err(|e| {
                        crate::error::DaemonError::Config(format!("elf base64 decode: {e}"))
                    })?;
                Ok((elf, *cycles))
            }
            _ => Err(crate::error::DaemonError::Config(
                "factory called with wrong JobKind".into(),
            )),
        }
    }

    pub struct BootRiverElfTest {
        target: heimdall_core::DutKind,
        elf: Vec<u8>,
        cycles: u64,
    }

    #[async_trait]
    impl Test for BootRiverElfTest {
        fn name(&self) -> &str {
            "boot-river-elf"
        }
        fn target(&self) -> heimdall_core::DutKind {
            self.target
        }
        async fn build(&self, _ctx: &mut BuildCtx<'_>) -> std::result::Result<Plan, TestError> {
            Ok(Plan {
                input: Artifact::new(ArtifactKind::ElfRiscv, self.elf.clone()),
                expected: State::new(),
                budget: StepBudget::cycles(self.cycles),
                inputs: std::collections::BTreeMap::new(),
            })
        }
    }
}

#[cfg(feature = "river")]
pub use river_factory::{BootRiverElfTest, RiverRealFactory};

#[cfg(all(test, feature = "river"))]
mod river_factory_tests {
    use super::*;
    use crate::dut_registry::{DutRecord, DutRegistry, IoPinmap, TransportSpec};
    use crate::types::{Job, JobKind, JobState};
    use base64::Engine;
    use chrono::Utc;
    use heimdall_core::{DutId, DutKind};
    use std::sync::Arc;

    #[tokio::test]
    async fn river_factory_builds_for_openocd_dut() {
        let mut registry = DutRegistry::new();
        registry.insert(DutRecord {
            id: DutId::new("river-1"),
            kind: DutKind::RiverRc1Nano,
            chip_serial: None,
            jtag: TransportSpec::Openocd {
                endpoint: "127.0.0.1:6666".parse().unwrap(),
            },
            pad_map: IoPinmap::default(),
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        });
        let factory = RiverRealFactory {
            registry: Arc::new(registry),
        };

        let elf_b64 = base64::engine::general_purpose::STANDARD.encode(b"\x7fELF...");
        let job = Job {
            id: crate::types::JobId::new(),
            dut: DutId::new("river-1"),
            dut_kind: DutKind::RiverRc1Nano,
            kind: JobKind::BootRiverElf {
                elf_b64,
                cycles: 1000,
            },
            campaign: None,
            state: JobState::Queued,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(factory.handles(&job.kind));
        let bundle = factory.build(&job).await.expect("build");
        assert_eq!(bundle.driver.target(), DutKind::RiverRc1Nano);
    }

    #[tokio::test]
    async fn river_factory_rejects_non_openocd_transport() {
        let mut registry = DutRegistry::new();
        registry.insert(DutRecord {
            id: DutId::new("river-1"),
            kind: DutKind::RiverRc1Nano,
            chip_serial: None,
            jtag: TransportSpec::Mock,
            pad_map: IoPinmap::default(),
            bringup: None,
            netlist: None,
            spice_watches: vec![],
        });
        let factory = RiverRealFactory {
            registry: Arc::new(registry),
        };
        let elf_b64 = base64::engine::general_purpose::STANDARD.encode(b"\x7fELF");
        let job = Job {
            id: crate::types::JobId::new(),
            dut: DutId::new("river-1"),
            dut_kind: DutKind::RiverRc1Nano,
            kind: JobKind::BootRiverElf { elf_b64, cycles: 1 },
            campaign: None,
            state: JobState::Queued,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let err = factory.build(&job).await.expect_err("should reject");
        assert!(
            format!("{err}").contains("only supports openocd"),
            "got: {err}"
        );
    }
}
