use std::ops::Sub;

use config::Config;

/// Utility data structure to display one of several strings based on a value in
/// a range, like a volume icon.
#[derive(Clone)]
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
        let prop = (f64::from(value) - f64::from(min))
            / (f64::from(max) - f64::from(min));
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
    pub fn parse(name: &str, global: &Config) -> Option<Self> {
        let ramps_table = global.get_table("ramps").ok()?;
        let ramp_table = ramps_table.get(name)?.clone().into_table().ok()?;
        let mut key = 0;
        let mut icons = Vec::new();
        while let Some(icon) = ramp_table.get(&key.to_string()) {
            if let Ok(icon) = icon.clone().into_string() {
                icons.push(icon);
                key += 1;
            } else {
                break;
            }
        }
        Some(Self { icons })
    }
}

impl FromIterator<String> for Ramp {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self {
            icons: iter.into_iter().collect(),
        }
    }
}
