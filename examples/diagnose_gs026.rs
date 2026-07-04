use ethers::abi::{decode, encode, ParamType, Token};
use ethers::types::Address;

fn main() -> anyhow::Result<()> {
    let signer: Address = "0x1111111111111111111111111111111111111111".parse()?;
    let expected_safe: Address = "0x2222222222222222222222222222222222222222".parse()?;
    let derived_safe: Address = "0x3333333333333333333333333333333333333333".parse()?;
    let owners = [signer, "0x4444444444444444444444444444444444444444".parse()?];

    println!("Offline GS026 diagnostic fixture");
    println!("signer:        {signer:?}");
    println!("expected safe: {expected_safe:?}");
    println!("derived safe:  {derived_safe:?}\n");

    if derived_safe != expected_safe {
        println!("address mismatch: derived safe differs from expected safe");
        println!("a live operator should resolve this before submitting a Safe transaction");
    }

    let encoded_owners = encode(&[Token::Array(
        owners.iter().copied().map(Token::Address).collect(),
    )]);
    let decoded = decode(
        &[ParamType::Array(Box::new(ParamType::Address))],
        &encoded_owners,
    )?;
    let decoded_owners = decoded[0]
        .clone()
        .into_array()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|token| token.into_address())
        .collect::<Vec<_>>();

    println!("fixture owners:");
    for owner in &decoded_owners {
        println!("  {owner:?}");
    }
    println!("signer owner check: {}", decoded_owners.contains(&signer));
    println!("No chain state was read.");

    Ok(())
}
