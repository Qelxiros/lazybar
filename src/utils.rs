use std::ops::Sub;

#[derive(Clone)]
pub struct Ramp {
    icons: Vec<String>,
}

impl Ramp {
    pub fn new(icons: Vec<String>) -> Self {
        Self { icons }
    }

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
}

impl FromIterator<String> for Ramp {
    fn from_iter<T: IntoIterator<Item = String>>(iter: T) -> Self {
        Self {
            icons: iter.into_iter().collect(),
        }
    }
}
