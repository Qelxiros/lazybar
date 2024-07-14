use std::ops::Sub;

use config::Config;

use crate::remove_string_from_config;

/// Utility data structure to display one of several strings based on a value in
/// a range, like a volume icon.
#[derive(Clone, Debug)]
pub struct Ramp {
    icons: Vec<String>,
}

impl Ramp {
    /// Given a value and a range, chooses the appropriate icon.
    pub fn choose<T>(&self, value: T, min: T, max: T) -> String
    where
        T: Sub + Copy,
        f64: From<T>,
    {
        let min = f64::from(min);
        let max = f64::from(max);
        let mut prop = (f64::from(value) - min) / (max - min);
        if prop < min {
            prop = min;
        }
        if prop > max {
            prop = max;
        }
        let idx = prop * (self.icons.len()) as f64;
        self.icons
            .get((idx.trunc() as usize).min(self.icons.len() - 1))
            .unwrap()
            .clone()
    }

    /// Parses a new instance with a given name from the global [`Config`].
    ///
    /// Ramps should be defined in a table called `[ramps]`. Each ramp should be
    /// a table with keys ranging from 0 to any number. The values should be
    /// [pango] markup strings.
    #[must_use]
    pub fn parse(name: impl AsRef<str>, global: &Config) -> Option<Self> {
        let ramps_table = global.get_table("ramps").ok()?;
        let mut ramp_table =
            ramps_table.get(name.as_ref())?.clone().into_table().ok()?;
        let mut key = 0;
        let mut icons = Vec::new();
        while let Some(icon) =
            remove_string_from_config(&key.to_string(), &mut ramp_table)
        {
            icons.push(icon);
            key += 1;
        }
        Some(Self { icons })
    }
}

impl Default for Ramp {
    fn default() -> Self {
        Self {
            icons: vec![String::from("")],
        }
    }
}

impl FromIterator<String> for Ramp {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self {
            icons: iter.into_iter().collect(),
        }
    }
}
