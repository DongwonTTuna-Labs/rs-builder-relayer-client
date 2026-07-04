use polymarket_relayer::{redeem_neg_risk_positions, redeem_regular, RelayerTxType, Transaction};

fn main() {
    let proxy_mode = RelayerTxType::Proxy;
    let regular = redeem_regular([0x77; 32], &[1, 2]);
    let neg_risk = redeem_neg_risk_positions([0x88; 32], &[1, 2]);

    println!("Offline proxy-wallet redemption plan");
    println!(
        "signature_type={} ({})",
        proxy_mode.signature_type(),
        proxy_mode.as_str()
    );
    println!("Transactions are printed for review only; nothing is submitted.\n");
    print_transaction("proxy regular redeem", &regular);
    print_transaction("proxy negative-risk redeem", &neg_risk);
}

fn print_transaction(label: &str, tx: &Transaction) {
    println!("{label}");
    println!("  to:    {}", tx.to);
    println!("  value: {}", tx.value);
    println!("  data:  {}", preview(&tx.data));
}

fn preview(value: &str) -> String {
    if value.len() <= 74 {
        return value.to_owned();
    }

    format!("{}...{}", &value[..42], &value[value.len() - 16..])
}
