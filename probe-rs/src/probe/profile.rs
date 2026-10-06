//! Probe profiles: how a debug probe's adapter is wired.
//!
//! Some debug probes are generic adapter chips whose pins can be wired up in many different ways.
//! FTDI MPSSE chips, for example, are used in many commercial JTAG adapters and on many
//! development boards, and each of them connects the reset lines, output buffers and LEDs to
//! different GPIO pins. A [`ProbeProfile`] describes this wiring, independently of the target
//! chip.
//!
//! Profiles are written in YAML. The FTDI section follows OpenOCD's `ftdi layout_init` and
//! `ftdi layout_signal` commands, so that existing OpenOCD interface configurations can be
//! translated directly:
//!
//! ```yaml
//! name: axm0432
//! description: Axiom AXM-0432 (e.g. Redbee Econotag)
//! ftdi:
//!   layout_init:
//!     data: 0x0c08
//!     direction: 0x0c2b
//!   signals:
//!     ntrst:
//!       data: 0x0800
//!     nsrst:
//!       data: 0x0400
//! ```
//!
//! probe-rs ships a set of built-in profiles (see [`ProbeProfile::builtins`]). A built-in profile
//! with `usb` matchers is applied automatically when a matching probe is opened. Any profile can
//! be applied explicitly with [`Probe::apply_profile`](crate::probe::Probe::apply_profile).

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

/// The YAML sources of the built-in probe profiles.
const BUILTIN_PROFILE_SOURCES: &[&str] = &[
    include_str!("../../probes/axm0432.yaml"),
    include_str!("../../probes/digilent-hs1.yaml"),
    include_str!("../../probes/digilent-hs2.yaml"),
    include_str!("../../probes/digilent-hs3.yaml"),
];

static BUILTIN_PROFILES: LazyLock<Vec<ProbeProfile>> = LazyLock::new(|| {
    BUILTIN_PROFILE_SOURCES
        .iter()
        .map(|yaml| ProbeProfile::from_yaml(yaml).expect("built-in probe profile is invalid"))
        .collect()
});

/// An error loading a [`ProbeProfile`].
#[derive(Debug, thiserror::Error, docsplay::Display)]
pub enum ProbeProfileError {
    /// The probe profile could not be parsed.
    Yaml(#[from] yaml_serde::Error),

    /// The probe profile '{name}' is invalid: {reason}
    Invalid {
        /// The name of the profile.
        name: String,
        /// Why the profile is invalid.
        reason: String,
    },

    /// There is no built-in probe profile named '{0}'.
    UnknownBuiltin(String),
}

/// Describes how a debug probe's adapter is wired.
///
/// See the [module documentation](self) for the file format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeProfile {
    /// The name of the profile, used to select it, e.g. with `--probe-profile`.
    pub name: String,

    /// A human readable description of the adapter(s) this profile is for.
    #[serde(default)]
    pub description: Option<String>,

    /// USB devices this profile applies to automatically.
    ///
    /// Leave this empty for adapters that use a generic VID/PID and product string, so that the
    /// profile is only applied when selected explicitly.
    #[serde(default)]
    pub usb: Vec<UsbMatch>,

    /// The wiring of an FTDI MPSSE based adapter.
    #[serde(default)]
    pub ftdi: Option<FtdiProfile>,
}

impl ProbeProfile {
    /// Parses and validates a probe profile from YAML.
    pub fn from_yaml(yaml: &str) -> Result<Self, ProbeProfileError> {
        let profile: Self = yaml_serde::from_str(yaml)?;
        profile.validate()?;
        Ok(profile)
    }

    /// Returns all built-in probe profiles.
    pub fn builtins() -> &'static [ProbeProfile] {
        &BUILTIN_PROFILES
    }

    /// Returns the built-in probe profile with the given name.
    pub fn builtin(name: &str) -> Result<&'static ProbeProfile, ProbeProfileError> {
        Self::builtins()
            .iter()
            .find(|profile| profile.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| ProbeProfileError::UnknownBuiltin(name.to_string()))
    }

    /// Returns the first built-in probe profile that applies to the given USB device.
    pub fn builtin_for_usb(
        vendor_id: u16,
        product_id: u16,
        product: Option<&str>,
    ) -> Option<&'static ProbeProfile> {
        Self::builtins().iter().find(|profile| {
            profile
                .usb
                .iter()
                .any(|usb| usb.matches(vendor_id, product_id, product))
        })
    }

    fn validate(&self) -> Result<(), ProbeProfileError> {
        let invalid = |reason: String| ProbeProfileError::Invalid {
            name: self.name.clone(),
            reason,
        };

        if self.name.is_empty() {
            return Err(invalid("the name is empty".to_string()));
        }

        if let Some(ftdi) = &self.ftdi {
            ftdi.validate().map_err(invalid)?;
        }

        Ok(())
    }
}

/// Matches a USB device by its VID, PID and, optionally, product string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsbMatch {
    /// The USB vendor ID.
    pub vendor_id: u16,
    /// The USB product ID.
    pub product_id: u16,
    /// The USB product string. If unset, any product string matches.
    #[serde(default)]
    pub product: Option<String>,
}

impl UsbMatch {
    /// Returns whether the given USB device matches.
    pub fn matches(&self, vendor_id: u16, product_id: u16, product: Option<&str>) -> bool {
        self.vendor_id == vendor_id
            && self.product_id == product_id
            && self
                .product
                .as_deref()
                .is_none_or(|expected| product == Some(expected))
    }
}

/// The wiring of an FTDI MPSSE based adapter.
///
/// Pins are given as 16-bit masks: bits 0-7 are the low byte (ADBUS0-7 on channel A), bits 8-15
/// are the high byte (ACBUS0-7 on channel A). Bits 0-3 are TCK, TDI, TDO and TMS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FtdiProfile {
    /// The initial pin levels and directions, like OpenOCD's `ftdi layout_init`.
    pub layout_init: FtdiPins,

    /// The adapter's control signals, like OpenOCD's `ftdi layout_signal`.
    #[serde(default)]
    pub signals: FtdiSignals,
}

/// The mask of the JTAG pins TCK, TDI, TDO and TMS.
const FTDI_JTAG_PINS: u16 = 0x000f;
/// The JTAG pins that must be outputs: TCK, TDI and TMS.
const FTDI_JTAG_OUTPUTS: u16 = 0x000b;

impl FtdiProfile {
    fn validate(&self) -> Result<(), String> {
        let direction = self.layout_init.direction;
        if direction & FTDI_JTAG_PINS != FTDI_JTAG_OUTPUTS {
            return Err(format!(
                "layout_init.direction must make TCK, TDI and TMS outputs and TDO an input \
                 (bits 0-3 = 0b1011), got {direction:#06x}"
            ));
        }

        for (name, signal) in self.signals.iter() {
            signal
                .validate()
                .map_err(|reason| format!("{name}: {reason}"))?;
        }

        Ok(())
    }
}

/// A set of FTDI pin levels and directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FtdiPins {
    /// The output levels. A set bit drives the pin high.
    pub data: u16,
    /// The pin directions. A set bit makes the pin an output.
    pub direction: u16,
}

/// The control signals of an FTDI adapter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FtdiSignals {
    /// The JTAG TAP reset (nTRST), active low.
    #[serde(default)]
    pub ntrst: Option<FtdiSignal>,
    /// The system reset (nSRST), active low.
    #[serde(default)]
    pub nsrst: Option<FtdiSignal>,
}

impl FtdiSignals {
    fn iter(&self) -> impl Iterator<Item = (&'static str, &FtdiSignal)> {
        [("ntrst", &self.ntrst), ("nsrst", &self.nsrst)]
            .into_iter()
            .filter_map(|(name, signal)| signal.as_ref().map(|signal| (name, signal)))
    }
}

/// How an FTDI adapter drives a signal, like OpenOCD's `ftdi layout_signal`.
///
/// A signal is driven through a data pin, an output enable pin, or both:
///
/// - `data`/`ndata` is the pin carrying the signal level, `ndata` if it is inverted (e.g. by a
///   transistor or inverting buffer).
/// - `oe`/`noe` is the pin enabling the signal's output buffer, `noe` if the enable is active
///   low. If the output enable mask equals the data mask, the data pin is tristated by switching
///   it to an input instead.
///
/// Signals are asserted by driving them low. They are deasserted by driving them high, or by
/// tristating them if `open_drain` is set or the signal has no data pin.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FtdiSignal {
    /// The data pin, non-inverted.
    #[serde(default)]
    pub data: u16,
    /// The data pin, inverted.
    #[serde(default)]
    pub ndata: u16,
    /// The output enable pin, active high.
    #[serde(default)]
    pub oe: u16,
    /// The output enable pin, active low.
    #[serde(default)]
    pub noe: u16,
    /// Whether to tristate the signal instead of driving it high when it is deasserted.
    #[serde(default)]
    pub open_drain: bool,
}

/// The level an [`FtdiSignal`] is set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SignalLevel {
    Low,
    High,
    HighZ,
}

impl FtdiSignal {
    fn data_mask(&self) -> u16 {
        self.data | self.ndata
    }

    fn oe_mask(&self) -> u16 {
        self.oe | self.noe
    }

    fn validate(&self) -> Result<(), String> {
        if self.data != 0 && self.ndata != 0 {
            return Err("only one of `data` and `ndata` can be set".to_string());
        }
        if self.oe != 0 && self.noe != 0 {
            return Err("only one of `oe` and `noe` can be set".to_string());
        }
        if self.data_mask() == 0 && self.oe_mask() == 0 {
            return Err("one of `data`, `ndata`, `oe` or `noe` must be set".to_string());
        }
        if (self.data_mask() | self.oe_mask()) & FTDI_JTAG_PINS != 0 {
            return Err("bits 0-3 are the JTAG pins and cannot be used for a signal".to_string());
        }
        if self.open_drain && self.oe_mask() == 0 {
            return Err("`open_drain` needs an output enable pin (`oe` or `noe`); \
                 set `oe` to the data mask to tristate the data pin"
                .to_string());
        }
        Ok(())
    }

    /// The level that deasserts this signal.
    pub(crate) fn deasserted_level(&self) -> SignalLevel {
        if self.open_drain || self.data_mask() == 0 {
            SignalLevel::HighZ
        } else {
            SignalLevel::High
        }
    }

    /// Applies `level` to the pin word `(data, direction)`.
    pub(crate) fn apply(&self, level: SignalLevel, pins: &mut FtdiPins) {
        let (high, enabled) = match level {
            SignalLevel::Low => (false, true),
            SignalLevel::High => (true, true),
            SignalLevel::HighZ => (false, false),
        };

        let data_mask = self.data_mask();
        let data_high = high != (self.ndata != 0);
        set_mask(&mut pins.data, data_mask, data_high);

        let oe_mask = self.oe_mask();
        if oe_mask == 0 {
            // Without an output enable, the data pin is always driven.
            set_mask(&mut pins.direction, data_mask, true);
        } else if oe_mask == data_mask {
            // The data pin is tristated by making it an input.
            set_mask(&mut pins.direction, data_mask, enabled);
        } else {
            set_mask(&mut pins.data, oe_mask, enabled != (self.noe != 0));
            set_mask(&mut pins.direction, data_mask | oe_mask, true);
        }
    }
}

fn set_mask(word: &mut u16, mask: u16, set: bool) {
    if set {
        *word |= mask;
    } else {
        *word &= !mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_are_valid() {
        assert!(!ProbeProfile::builtins().is_empty());
        for profile in ProbeProfile::builtins() {
            assert!(
                ProbeProfile::builtin(&profile.name).is_ok(),
                "{} is ambiguous",
                profile.name
            );
        }
    }

    #[test]
    fn digilent_is_matched_by_product_string() {
        let hs3 = ProbeProfile::builtin_for_usb(0x0403, 0x6014, Some("Digilent USB Device"));
        assert_eq!(hs3.map(|p| p.name.as_str()), Some("digilent-hs3"));

        let generic = ProbeProfile::builtin_for_usb(0x0403, 0x6010, Some("Dual RS232-HS"));
        assert_eq!(generic, None);
    }

    #[test]
    fn rejects_unknown_fields() {
        let yaml = "name: x\nftdi:\n  layout_init: { data: 8, direction: 11 }\n  signals:\n    led: { data: 0x10 }\n";
        assert!(matches!(
            ProbeProfile::from_yaml(yaml),
            Err(ProbeProfileError::Yaml(_))
        ));
    }

    #[test]
    fn rejects_signal_on_jtag_pin() {
        let yaml = "name: x\nftdi:\n  layout_init: { data: 8, direction: 11 }\n  signals:\n    nsrst: { data: 0x01 }\n";
        assert!(matches!(
            ProbeProfile::from_yaml(yaml),
            Err(ProbeProfileError::Invalid { .. })
        ));
    }

    #[test]
    fn rejects_jtag_pin_directions() {
        let yaml = "name: x\nftdi:\n  layout_init: { data: 8, direction: 0x0f }\n";
        assert!(matches!(
            ProbeProfile::from_yaml(yaml),
            Err(ProbeProfileError::Invalid { .. })
        ));
    }

    fn pins() -> FtdiPins {
        FtdiPins {
            data: 0,
            direction: FTDI_JTAG_OUTPUTS,
        }
    }

    #[test]
    fn data_signal_is_push_pull() {
        let signal = FtdiSignal {
            data: 0x0400,
            ..Default::default()
        };
        let mut p = pins();
        signal.apply(SignalLevel::Low, &mut p);
        assert_eq!((p.data, p.direction), (0x0000, 0x040b));
        signal.apply(signal.deasserted_level(), &mut p);
        assert_eq!((p.data, p.direction), (0x0400, 0x040b));
    }

    #[test]
    fn inverted_data_signal() {
        let signal = FtdiSignal {
            ndata: 0x0010,
            ..Default::default()
        };
        let mut p = pins();
        signal.apply(SignalLevel::Low, &mut p);
        assert_eq!(p.data, 0x0010);
        signal.apply(SignalLevel::High, &mut p);
        assert_eq!(p.data, 0x0000);
    }

    #[test]
    fn oe_only_signal_is_open_drain() {
        // Olimex style nSRST: a transistor pulls the line low while the pin is high.
        let signal = FtdiSignal {
            oe: 0x0200,
            ..Default::default()
        };
        assert_eq!(signal.deasserted_level(), SignalLevel::HighZ);
        let mut p = pins();
        signal.apply(SignalLevel::Low, &mut p);
        assert_eq!((p.data, p.direction), (0x0200, 0x020b));
        signal.apply(SignalLevel::HighZ, &mut p);
        assert_eq!((p.data, p.direction), (0x0000, 0x020b));
    }

    #[test]
    fn data_signal_tristated_by_direction() {
        let signal = FtdiSignal {
            data: 0x0100,
            oe: 0x0100,
            open_drain: true,
            ..Default::default()
        };
        let mut p = pins();
        signal.apply(SignalLevel::Low, &mut p);
        assert_eq!((p.data, p.direction), (0x0000, 0x010b));
        signal.apply(signal.deasserted_level(), &mut p);
        assert_eq!((p.data, p.direction), (0x0000, 0x000b));
    }

    #[test]
    fn data_signal_with_buffer_enable() {
        // Olimex style nTRST: data pin behind a buffer enabled by an active low pin.
        let signal = FtdiSignal {
            data: 0x0100,
            noe: 0x0400,
            ..Default::default()
        };
        let mut p = pins();
        signal.apply(SignalLevel::Low, &mut p);
        assert_eq!((p.data, p.direction), (0x0000, 0x050b));
        signal.apply(SignalLevel::HighZ, &mut p);
        assert_eq!((p.data, p.direction), (0x0400, 0x050b));
    }
}
