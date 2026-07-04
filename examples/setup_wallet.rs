use polymarket_relayer::{
    approve_ctf_for_ctf_exchange, approve_ctf_for_neg_risk_adapter,
    approve_ctf_for_neg_risk_exchange, approve_usdc_for_ctf_exchange,
    approve_usdc_for_neg_risk_exchange, Transaction,
};

fn main() {
    let plan = [
        ("USDC -> CTF Exchange", approve_usdc_for_ctf_exchange()),
        ("USDC -> Neg Risk Exchange", approve_usdc_for_neg_risk_exchange()),
        ("CTF -> CTF Exchange", approve_ctf_for_ctf_exchange()),
        ("CTF -> Neg Risk Exchange", approve_ctf_for_neg_risk_exchange()),
        ("CTF -> Neg Risk Adapter", approve_ctf_for_neg_risk_adapter()),
    ];

    println!("Offline Safe setup approval plan");
    println!("No wallet deployment or approval submission is performed.\n");

    for (label, tx) in plan {
        print_transaction(label, &tx);
    }
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
