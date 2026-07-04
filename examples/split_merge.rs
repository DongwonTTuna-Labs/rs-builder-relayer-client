use ethers::types::U256;
use polymarket_relayer::{merge_regular, split_regular, Transaction};

fn main() {
    let condition_id = [0x33; 32];
    let partition = [1, 2];
    let amount = U256::from(1_000_000u64);

    let split = split_regular(condition_id, &partition, amount);
    let merge = merge_regular(condition_id, &partition, amount);

    println!("Offline split/merge plan");
    println!("Amount uses 6-decimal USDC units: {amount}");
    println!("Transactions are printed for review only; nothing is submitted.\n");
    print_transaction("split position", &split);
    print_transaction("merge position", &merge);
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
