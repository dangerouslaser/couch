//! Explicit activity membership and executable on/off sequences. Legacy `steps`
//! remain untouched and are never silently promoted into executable commands.
use crate::{Action, Config, DeviceId};
use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActivitySetup {
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub devices: Vec<DeviceId>,
    #[serde(default)]
    pub keep_awake: bool,
    #[serde(default)]
    pub on: Vec<SequenceStep>,
    #[serde(default)]
    pub off: Vec<SequenceStep>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum SequenceStep {
    Command { action: Action },
    Delay { ms: u32 },
}
impl ActivitySetup {
    pub fn validate(&self, config: &Config) -> Result<(), &'static str> {
        if self.description.len() > 1024 {
            return Err("Description must be at most 1024 bytes");
        }
        for (i, id) in self.devices.iter().enumerate() {
            if self.devices[..i].contains(id) || !config.devices().any(|(_, d)| &d.id == id) {
                return Err("Included devices must exist and appear only once");
            }
        }
        for steps in [&self.on, &self.off] {
            if steps.len() > 64 {
                return Err("A sequence supports at most 64 steps");
            }
            let mut wait = 0u32;
            for step in steps {
                match step {
                    SequenceStep::Delay { ms } => {
                        if *ms == 0 || *ms > 30_000 {
                            return Err("Delays must be between 1 and 30000 ms");
                        }
                        wait += ms;
                    }
                    SequenceStep::Command { action } => {
                        let valid = self.devices.contains(&action.device)
                            && config
                                .devices()
                                .find(|(_, d)| d.id == action.device)
                                .and_then(|(_, d)| config.resolve_integration(&d.integration))
                                .is_some_and(|i| {
                                    crate::commands::Function::parse(&action.command)
                                        .is_some_and(|f| f.supports(&i))
                                });
                        if !valid {
                            return Err("Choose a supported command from an included device");
                        }
                    }
                }
            }
            if wait > 120_000 {
                return Err("Total delay must not exceed two minutes per sequence");
            }
        }
        Ok(())
    }
    pub fn forget_device(&mut self, id: &DeviceId) {
        self.devices.retain(|d| d != id);
        for steps in [&mut self.on, &mut self.off] {
            steps
                .retain(|s| !matches!(s, SequenceStep::Command { action } if &action.device == id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sequences_require_supported_commands_from_members_and_bound_delays() {
        let config = Config::seed();
        let mut setup = ActivitySetup {
            devices: alloc::vec!["living-kodi".into()],
            on: alloc::vec![
                SequenceStep::Command {
                    action: Action::new("living-kodi", "home")
                },
                SequenceStep::Delay { ms: 1000 }
            ],
            ..Default::default()
        };
        assert!(setup.validate(&config).is_ok());
        setup.devices.clear();
        assert!(setup.validate(&config).is_err());
        setup.devices.push("living-kodi".into());
        setup.on[0] = SequenceStep::Command {
            action: Action::new("living-kodi", "power-on"),
        };
        assert!(setup.validate(&config).is_err());
        setup.on = alloc::vec![SequenceStep::Delay { ms: 30001 }];
        assert!(setup.validate(&config).is_err());
        setup.on = alloc::vec![SequenceStep::Delay{ms:30000};5];
        assert!(setup.validate(&config).is_err());
    }
    #[test]
    fn config_device_removal_cleans_both_lifecycle_sequences() {
        let mut config = Config::seed();
        config.activities[0].setup = ActivitySetup {
            devices: alloc::vec!["living-kodi".into()],
            on: alloc::vec![SequenceStep::Command {
                action: Action::new("living-kodi", "home")
            }],
            off: alloc::vec![SequenceStep::Command {
                action: Action::new("living-kodi", "stop")
            }],
            ..Default::default()
        };
        config
            .remove_device(&"living-room".into(), &"living-kodi".into())
            .unwrap();
        assert!(config.activities[0].setup.devices.is_empty());
        assert!(config.activities[0].setup.on.is_empty());
        assert!(config.activities[0].setup.off.is_empty());
        assert!(config.validate().is_ok());
    }
    #[test]
    fn removing_device_cleans_sequences_but_preserves_delays() {
        let mut setup = ActivitySetup {
            devices: alloc::vec!["player".into()],
            on: alloc::vec![
                SequenceStep::Command {
                    action: Action::new("player", "home")
                },
                SequenceStep::Delay { ms: 1000 }
            ],
            ..Default::default()
        };
        setup.forget_device(&"player".into());
        assert!(setup.devices.is_empty());
        assert_eq!(setup.on, alloc::vec![SequenceStep::Delay { ms: 1000 }]);
    }
}
