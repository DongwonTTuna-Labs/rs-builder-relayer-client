use std::fmt;

use ethers::types::U256;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::error::{RelayerError, Result};

use super::config::PUSD_DECIMALS;

/// CTF outcome-position quantity, which is 1:1 with collateral base units.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CtfPositionAmount {
    base_units: U256,
}

impl CtfPositionAmount {
    /// Construct a CTF position amount from six-decimal collateral base units.
    pub fn from_base_units(base_units: U256) -> Result<Self> {
        if base_units.is_zero() {
            return Err(RelayerError::Other(
                "CTF position amount must be non-zero".to_string(),
            ));
        }

        Ok(Self { base_units })
    }

    /// Return the position quantity in collateral base units.
    pub fn base_units(&self) -> U256 {
        self.base_units
    }

    /// Return the collateral decimal precision, always 6.
    pub fn decimals(&self) -> u8 {
        PUSD_DECIMALS
    }

    #[cfg(test)]
    pub(super) fn unchecked_for_test(base_units: U256) -> Self {
        Self { base_units }
    }
}

impl Serialize for CtfPositionAmount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("CtfPositionAmount", 2)?;
        state.serialize_field("base_units", &self.base_units.to_string())?;
        state.serialize_field("decimals", &PUSD_DECIMALS)?;
        state.end()
    }
}

impl fmt::Debug for CtfPositionAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CtfPositionAmount")
            .field("base_units", &self.base_units.to_string())
            .field("decimals", &PUSD_DECIMALS)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn ctf_position_amount_rejects_zero_and_serializes_decimal_base_units() {
        assert_eq!(
            CtfPositionAmount::from_base_units(U256::zero())
                .expect_err("zero CTF position amount must fail")
                .to_string(),
            "CTF position amount must be non-zero"
        );

        let amount = CtfPositionAmount::from_base_units(U256::from(1_000_000u64))
            .expect("one collateral unit should validate");
        assert_eq!(amount.base_units(), U256::from(1_000_000u64));
        assert_eq!(amount.decimals(), PUSD_DECIMALS);
        assert_eq!(
            serde_json::to_value(amount).expect("position amount should serialize"),
            json!({
                "base_units": "1000000",
                "decimals": 6,
            })
        );
        assert_eq!(
            format!("{amount:?}"),
            "CtfPositionAmount { base_units: \"1000000\", decimals: 6 }"
        );
    }
}
