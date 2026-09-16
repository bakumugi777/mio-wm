use std::{error::Error, fmt};

/// Stable identity of a display looking into Mio's shared World.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OutputId(u64);

impl OutputId {
    #[must_use]
    pub const fn from_raw(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputError {
    Unknown(OutputId),
    IdExhausted,
}

impl fmt::Display for OutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(formatter, "unknown output {}", id.get()),
            Self::IdExhausted => formatter.write_str("output ID space exhausted"),
        }
    }
}

impl Error for OutputError {}
