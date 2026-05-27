use crate::error::ConfigError;
use crate::schema::ConfigFile;
use std::collections::HashSet;

pub fn validate(cfg: &ConfigFile) -> Result<(), ConfigError> {
    // No duplicate transport ids
    let mut seen_transport = HashSet::new();
    for t in &cfg.transport.jtag {
        if !seen_transport.insert(&t.id) {
            return Err(ConfigError::DuplicateTransportId(t.id.clone()));
        }
    }
    for t in &cfg.transport.uart {
        if !seen_transport.insert(&t.id) {
            return Err(ConfigError::DuplicateTransportId(t.id.clone()));
        }
    }
    for t in &cfg.transport.usb {
        if !seen_transport.insert(&t.id) {
            return Err(ConfigError::DuplicateTransportId(t.id.clone()));
        }
    }
    for t in &cfg.transport.psu {
        if !seen_transport.insert(&t.id) {
            return Err(ConfigError::DuplicateTransportId(t.id.clone()));
        }
    }
    let mut seen_gpio = HashSet::new();
    for t in &cfg.transport.gpio {
        if !seen_gpio.insert(&t.id) {
            return Err(ConfigError::DuplicateGpioTransportId(t.id.clone()));
        }
        if !seen_transport.insert(&t.id) {
            return Err(ConfigError::DuplicateTransportId(t.id.clone()));
        }
    }

    // No duplicate dut ids; transport refs resolve
    let mut seen_dut = HashSet::new();
    for d in &cfg.duts {
        if !seen_dut.insert(&d.id) {
            return Err(ConfigError::DuplicateDutId(d.id.clone()));
        }
        for tref in &d.transports {
            if !seen_transport.contains(&tref) {
                return Err(ConfigError::UnknownTransportRef {
                    dut: d.id.clone(),
                    transport: tref.clone(),
                });
            }
        }
        // Golden backend must exist for the dut's family
        let needed = match d.kind.family() {
            heimdall_core::kind::Family::Fpga => cfg.golden.aegis.is_some(),
            heimdall_core::kind::Family::Cpu => cfg.golden.river.is_some(),
        };
        if !needed {
            return Err(ConfigError::MissingGoldenBackend {
                dut: d.id.clone(),
                kind: d.kind,
            });
        }
        // Spice watches: no duplicate heimdall names.
        let mut seen_watch = HashSet::new();
        for w in &d.spice_watches {
            if !seen_watch.insert(w.name.clone()) {
                return Err(ConfigError::DuplicateSpiceWatch {
                    dut: d.id.clone(),
                    name: w.name.clone(),
                });
            }
        }
    }

    // Validate pad_map entries
    let gpio_ids: HashSet<&String> = cfg.transport.gpio.iter().map(|g| &g.id).collect();
    let dut_ids: HashSet<&String> = cfg.duts.iter().map(|d| &d.id).collect();
    let mut seen_pad = HashSet::new();
    for entry in &cfg.pad_maps {
        if !dut_ids.contains(&entry.dut) {
            return Err(ConfigError::PadMapUnknownDut(entry.dut.clone()));
        }
        if !gpio_ids.contains(&entry.gpio_transport) {
            return Err(ConfigError::PadMapUnknownGpioTransport(
                entry.gpio_transport.clone(),
            ));
        }
        let key = (entry.dut.clone(), entry.direction, entry.fpga_pad);
        if !seen_pad.insert(key) {
            return Err(ConfigError::DuplicatePadMap {
                dut: entry.dut.clone(),
                direction: entry.direction,
                fpga_pad: entry.fpga_pad,
            });
        }
    }

    Ok(())
}
