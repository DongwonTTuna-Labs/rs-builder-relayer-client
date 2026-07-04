use polymarket_relayer::{redeem_neg_risk_positions, redeem_regular, Transaction};

struct FixturePosition {
    title: &'static str,
    condition_id: [u8; 32],
    negative_risk: bool,
    redeemable: bool,
    expected_usdc: u64,
}

fn main() {
    let positions = [
        FixturePosition {
            title: "Synthetic regular winner",
            condition_id: [0x44; 32],
            negative_risk: false,
            redeemable: true,
            expected_usdc: 18,
        },
        FixturePosition {
            title: "Synthetic negative-risk winner",
            condition_id: [0x55; 32],
            negative_risk: true,
            redeemable: true,
            expected_usdc: 7,
        },
        FixturePosition {
            title: "Synthetic active market",
            condition_id: [0x66; 32],
            negative_risk: false,
            redeemable: false,
            expected_usdc: 0,
        },
    ];

    let mut planned = Vec::new();
    let mut expected_total = 0u64;

    for position in positions {
        if !position.redeemable {
            println!("skip active fixture: {}", position.title);
            continue;
        }

        expected_total += position.expected_usdc;
        let tx = if position.negative_risk {
            redeem_neg_risk_positions(position.condition_id, &[1, 2])
        } else {
            redeem_regular(position.condition_id, &[1, 2])
        };
        planned.push((position.title, tx));
    }

    println!("\nOffline redeem-all plan");
    println!("{} synthetic redemption(s)", planned.len());
    println!("expected fixture payout: {expected_total} USDC");
    println!("Transactions are printed for review only; nothing is submitted.\n");

    for (title, tx) in planned {
        print_transaction(title, &tx);
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
