use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayerTransactionState {
    New,
    Executed,
    Mined,
    Confirmed,
    Invalid,
    Failed,
    Unknown(String),
}

impl RelayerTransactionState {
    pub fn parse(raw: &str) -> Self {
        let normalized = raw.to_uppercase();
        let state = normalized.strip_prefix("STATE_").unwrap_or(&normalized);

        match state {
            "NEW" => Self::New,
            "EXECUTED" => Self::Executed,
            "MINED" => Self::Mined,
            "CONFIRMED" => Self::Confirmed,
            "INVALID" => Self::Invalid,
            "FAILED" => Self::Failed,
            _ => Self::Unknown(raw.to_string()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::New => "New".to_string(),
            Self::Executed => "Executed".to_string(),
            Self::Mined => "Mined".to_string(),
            Self::Confirmed => "Confirmed".to_string(),
            Self::Invalid => "Invalid".to_string(),
            Self::Failed => "Failed".to_string(),
            Self::Unknown(raw) => format!("Unknown({raw})"),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Confirmed | Self::Invalid | Self::Failed)
    }

    pub fn is_success(&self) -> bool {
        matches!(self, Self::Mined | Self::Confirmed)
    }

    fn as_wire_str(&self) -> &str {
        match self {
            Self::New => "STATE_NEW",
            Self::Executed => "STATE_EXECUTED",
            Self::Mined => "STATE_MINED",
            Self::Confirmed => "STATE_CONFIRMED",
            Self::Invalid => "STATE_INVALID",
            Self::Failed => "STATE_FAILED",
            Self::Unknown(raw) => raw.as_str(),
        }
    }
}

impl Serialize for RelayerTransactionState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_wire_str())
    }
}

impl<'de> Deserialize<'de> for RelayerTransactionState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}
