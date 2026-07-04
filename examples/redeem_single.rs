use polymarket_relayer::{redeem_neg_risk_positions, redeem_regular, Transaction};

fn main() {
    let regular_condition = [0x11; 32];
    let neg_risk_condition = [0x22; 32];

    let regular = redeem_regular(regular_condition, &[1, 2]);
    let neg_risk = redeem_neg_risk_positions(neg_risk_condition, &[1, 2]);

    println!("Offline single redemption plan");
    println!("Transactions are printed for review only; nothing is submitted.\n");
    print_transaction("regular market redeem", &regular);
    print_transaction("negative-risk market redeem", &neg_risk);
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
