use ethers::types::Address;
use ethers::utils::to_checksum;
use serde::Serialize;

use crate::deposit_wallet::DepositWalletCall;

const REDACTION_MARKER: &str = "full calldata and signatures are intentionally omitted";

/// WALLET batch 후보 call 하나의 리뷰용 요약.
///
/// 전체 calldata는 담지 않으며, 4바이트 이하의 data는 전체 payload가 selector로
/// 재현되지 않도록 selector도 생략한다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BatchCallSummary {
    target: String,
    value: String,
    selector: Option<String>,
    data_len: usize,
}

impl BatchCallSummary {
    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn selector(&self) -> Option<&str> {
        self.selector.as_deref()
    }

    pub fn data_len(&self) -> usize {
        self.data_len
    }
}

/// WALLET batch 후보 call 묶음의 리뷰용 요약. full calldata를 담지 않는다.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DepositWalletBatchSummary {
    call_count: usize,
    total_calldata_bytes: usize,
    calls: Vec<BatchCallSummary>,
    redaction: String,
}

impl DepositWalletBatchSummary {
    pub fn call_count(&self) -> usize {
        self.call_count
    }

    pub fn total_calldata_bytes(&self) -> usize {
        self.total_calldata_bytes
    }

    pub fn calls(&self) -> &[BatchCallSummary] {
        &self.calls
    }

    pub fn redaction(&self) -> &str {
        &self.redaction
    }
}

/// Summarize candidate WALLET batch calls without validating the batch.
///
/// Call order is preserved. Empty batches and empty calldata are valid summary
/// inputs; signing and submit preflight remain responsible for validation.
pub fn summarize_batch_calls(calls: &[DepositWalletCall]) -> DepositWalletBatchSummary {
    let summaries = calls
        .iter()
        .map(|call| BatchCallSummary {
            target: redacted_address(call.target),
            value: call.value.to_string(),
            selector: (call.data.len() > 4)
                .then(|| format!("0x{}", hex::encode(&call.data[..4]))),
            data_len: call.data.len(),
        })
        .collect();

    DepositWalletBatchSummary {
        call_count: calls.len(),
        total_calldata_bytes: calls.iter().map(|call| call.data.len()).sum(),
        calls: summaries,
        redaction: REDACTION_MARKER.to_string(),
    }
}

fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}

#[cfg(test)]
mod tests {
    use ethers::types::{Bytes, U256};

    use super::*;

    #[test]
    fn summary_exposes_only_review_metadata_and_preserves_order() {
        let first = DepositWalletCall {
            target: Address::from_low_u64_be(1),
            value: U256::from(7u64),
            data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef, 0x01]),
        };
        let second = DepositWalletCall {
            target: Address::from_low_u64_be(2),
            value: U256::zero(),
            data: Bytes::new(),
        };

        let summary = summarize_batch_calls(&[first, second]);
        let json = serde_json::to_string(&summary).expect("summary should serialize");

        assert_eq!(summary.call_count(), 2);
        assert_eq!(summary.total_calldata_bytes(), 5);
        assert_eq!(summary.calls()[0].value(), "7");
        assert_eq!(summary.calls()[0].selector(), Some("0xdeadbeef"));
        assert_eq!(summary.calls()[1].selector(), None);
        assert_eq!(summary.calls()[1].data_len(), 0);
        assert_eq!(summary.redaction(), REDACTION_MARKER);
        assert!(!json.contains("deadbeef01"));
    }

    #[test]
    fn four_byte_data_does_not_become_replayable_selector_output() {
        let call = DepositWalletCall {
            target: Address::from_low_u64_be(1),
            value: U256::zero(),
            data: Bytes::from(vec![0xde, 0xad, 0xbe, 0xef]),
        };

        let summary = summarize_batch_calls(&[call]);
        let json = serde_json::to_string(&summary).expect("summary should serialize");
        let debug = format!("{summary:?}");

        assert_eq!(summary.calls()[0].selector(), None);
        assert_eq!(summary.calls()[0].data_len(), 4);
        assert!(!json.contains("deadbeef"));
        assert!(!debug.contains("deadbeef"));
    }
}
