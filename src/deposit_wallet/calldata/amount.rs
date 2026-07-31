use std::fmt;

use ethers::types::U256;
use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

use crate::error::{RelayerError, Result};

use super::config::PUSD_DECIMALS;

/// pUSD amount expressed in 6-decimal base units.
///
/// Construction is explicit so callers cannot pass an ambiguous raw integer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PusdAmount {
    base_units: U256,
}

impl PusdAmount {
    /// Construct an amount from pUSD base units (10^-6 pUSD).
    ///
    /// Zero is rejected because approval cancellation is outside this API.
    pub fn from_base_units(base_units: U256) -> Result<Self> {
        if base_units.is_zero() {
            return Err(RelayerError::Other(
                "pUSD amount must be non-zero".to_string(),
            ));
        }

        Ok(Self { base_units })
    }

    /// Convert whole pUSD into 6-decimal base units.
    ///
    /// Zero is rejected. Every `u64` input fits after conversion through
    /// `u128`, so this constructor has no unreachable overflow branch.
    pub fn from_whole_pusd(whole: u64) -> Result<Self> {
        let base_units =
            u128::from(whole) * 10u128.pow(u32::from(PUSD_DECIMALS));
        Self::from_base_units(U256::from(base_units))
    }

    /// Express `uint256::MAX` as an explicit approval amount.
    ///
    /// This constructor only provides a representation. It does not choose an
    /// unlimited-approval policy; that decision remains with later policy work.
    pub fn unlimited() -> Self {
        Self {
            base_units: U256::MAX,
        }
    }

    /// Return the amount in pUSD base units.
    pub fn base_units(&self) -> U256 {
        self.base_units
    }

    /// Return the pUSD decimal precision, always 6.
    pub fn decimals(&self) -> u8 {
        PUSD_DECIMALS
    }

    /// Return whether this amount is the explicit `uint256::MAX` value.
    pub fn is_unlimited(&self) -> bool {
        self.base_units == U256::MAX
    }
}

impl Serialize for PusdAmount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("PusdAmount", 2)?;
        state.serialize_field("base_units", &self.base_units.to_string())?;
        state.serialize_field("decimals", &PUSD_DECIMALS)?;
        state.end()
    }
}

impl fmt::Debug for PusdAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PusdAmount")
            .field("base_units", &self.base_units.to_string())
            .field("decimals", &PUSD_DECIMALS)
            .finish()
    }
}

#[cfg(test)]
impl PusdAmount {
    pub(super) fn unchecked_for_test(base_units: U256) -> Self {
        Self { base_units }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn pusd_amount_rejects_zero_and_converts_whole_units() {
        assert_eq!(
            PusdAmount::from_base_units(U256::zero())
                .expect_err("zero base units must be rejected")
                .to_string(),
            "pUSD amount must be non-zero"
        );
        assert_eq!(
            PusdAmount::from_whole_pusd(0)
                .expect_err("zero whole pUSD must be rejected")
                .to_string(),
            "pUSD amount must be non-zero"
        );

        let one = PusdAmount::from_whole_pusd(1).expect("one pUSD should convert");
        assert_eq!(one.base_units(), U256::from(1_000_000u64));
        assert_eq!(one.decimals(), PUSD_DECIMALS);
        assert!(!one.is_unlimited());

        let max_whole = PusdAmount::from_whole_pusd(u64::MAX)
            .expect("u64::MAX whole pUSD fits through u128");
        let expected_max_whole =
            U256::from(u128::from(u64::MAX) * 10u128.pow(u32::from(PUSD_DECIMALS)));
        assert_eq!(max_whole.base_units(), expected_max_whole);

        let unlimited = PusdAmount::unlimited();
        assert_eq!(unlimited.base_units(), U256::MAX);
        assert_eq!(unlimited.decimals(), PUSD_DECIMALS);
        assert!(unlimited.is_unlimited());
    }

    #[test]
    fn pusd_amount_serializes_decimal_base_units_with_decimals() {
        let amount = PusdAmount::from_whole_pusd(1).expect("one pUSD should convert");
        let serialized = serde_json::to_value(amount).expect("amount should serialize");

        assert_eq!(
            serialized,
            json!({
                "base_units": "1000000",
                "decimals": 6,
            })
        );
        assert_eq!(
            format!("{amount:?}"),
            "PusdAmount { base_units: \"1000000\", decimals: 6 }"
        );

        let unlimited = serde_json::to_value(PusdAmount::unlimited())
            .expect("unlimited amount should serialize");
        assert_eq!(
            unlimited["base_units"],
            "115792089237316195423570985008687907853269984665640564039457584007913129639935"
        );
        assert_eq!(unlimited["decimals"], 6);
    }
}
