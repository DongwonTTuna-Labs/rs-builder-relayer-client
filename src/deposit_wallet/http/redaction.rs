use super::*;

pub(super) fn sanitized_external_token(value: &str) -> String {
    external_token_hash(value)
}

pub(super) fn external_token_hash(value: &str) -> String {
    let hex = hex::encode(keccak256(value.as_bytes()));
    format!("sha3:0x{}...{}", &hex[..8], &hex[56..])
}

pub(super) fn unknown_state_error_summary(_value: &str) -> &'static str {
    "<unrecognized relayer state>"
}

pub(super) fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}
