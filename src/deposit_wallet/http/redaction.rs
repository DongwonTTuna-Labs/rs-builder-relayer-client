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

pub(super) fn payload_hash_summary(bytes: &[u8]) -> String {
    let hex = hex::encode(keccak256(bytes));
    format!("0x{hex}")
}

pub(super) fn signed_digest_payload_hash(digest: H256) -> String {
    let hex = hex::encode(digest.as_bytes());
    format!("signed-digest:0x{hex}")
}

pub(super) fn recovered_payload_hash(transaction_id: &str) -> String {
    let hex = hex::encode(keccak256(transaction_id.as_bytes()));
    format!("recovered:0x{hex}")
}

pub(super) fn display_payload_hash(payload_hash: &str) -> String {
    let Some((prefix, hex)) = payload_hash.rsplit_once("0x") else {
        return external_token_hash(payload_hash);
    };
    if hex.len() != 64 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return external_token_hash(payload_hash);
    }
    format!("{prefix}0x{}...{}", &hex[..8], &hex[56..])
}

pub(super) fn redacted_address(address: Address) -> String {
    let checksum = to_checksum(&address, None);
    format!("{}...{}", &checksum[..6], &checksum[38..])
}
