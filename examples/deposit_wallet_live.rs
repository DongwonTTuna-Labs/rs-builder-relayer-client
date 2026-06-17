use std::env;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail, Context, Result};
use ethers::types::{Address, U256};
use ethers::utils::{keccak256, to_checksum};
use polymarket_relayer::deposit_wallet::{
    build_deposit_wallet_batch_request_from_signed, build_erc20_approve_call,
    deposit_wallet_contract_config, DepositWalletCall, DepositWalletContractConfig,
    DepositWalletDeployment, DepositWalletOwnerSigner, DepositWalletPollingConfig,
    DepositWalletRelayerClient, DepositWalletRelayerUrl, RelayerKeyAuth,
    RelayerTransactionState, SignedDepositWalletBatch, AMOY_CHAIN_ID,
    POLYMARKET_OWNER_PRIVATE_KEY_ENV,
};
use polymarket_relayer::RelayerError;

const DEFAULT_NETWORK: &str = "amoy";
const AMOY_RELAYER_URL_ENV: &str = "POLYMARKET_AMOY_RELAYER_URL";
const DEFAULT_AMOY_RELAYER_URL: &str = "https://relayer-v2-staging.polymarket.dev/";
const LIVE_GATE_ENV: &str = "POLYMARKET_RELAYER_ALLOW_LIVE_AMOY";
const RELAYER_API_KEY_ENV: &str = "POLYMARKET_RELAYER_API_KEY";
const RELAYER_API_KEY_ADDRESS_ENV: &str = "POLYMARKET_RELAYER_API_KEY_ADDRESS";
const OWNER_ADDRESS_ENV: &str = "POLYMARKET_OWNER_ADDRESS";
const APPROVE_TOKEN_ENV: &str = "POLYMARKET_AMOY_APPROVE_TOKEN";
const APPROVE_SPENDER_ENV: &str = "POLYMARKET_AMOY_APPROVE_SPENDER";
const APPROVE_AMOUNT_ENV: &str = "POLYMARKET_AMOY_APPROVE_AMOUNT";
const DEADLINE_VALID_FOR: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("Outcome: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let _ = dotenvy::dotenv();

    let cli = match parse_cli()? {
        CliAction::Run(cli) => cli,
        CliAction::Help => return Ok(()),
    };
    validate_network(&cli.network)?;

    print_header(&cli);
    if cli.execute {
        enforce_live_gate()?;
    } else {
        println!("Dry-run safety: no POST /submit calls will be made.");
    }

    let relayer_url_raw = load_relayer_url();
    if cli.execute {
        enforce_live_relayer_url(&relayer_url_raw)?;
    }
    let relayer_url = DepositWalletRelayerUrl::parse(&relayer_url_raw)
        .context("relayer URL failed the production allowlist guard")?;
    let contract_config = deposit_wallet_contract_config(AMOY_CHAIN_ID)
        .context("Amoy deposit wallet contract config is unavailable")?;

    let owner_signer = DepositWalletOwnerSigner::from_env()?;
    let owner = owner_signer.owner_address();
    validate_optional_owner_address(owner)?;

    let relayer_auth = load_relayer_auth()?;
    let approve = load_approve_config()?;
    let client = DepositWalletRelayerClient::new(relayer_url, relayer_auth, contract_config)?;
    let poll_config = DepositWalletPollingConfig::default();

    print_public_config(owner, &relayer_url_raw, &approve, poll_config);

    let deployment = client
        .discover_deposit_wallet(owner)
        .await
        .map_err(|error| classify_read_error("wallet discovery", error))?;
    let deposit_wallet = deployment.deposit_wallet();
    print_deployment(&deployment);

    if cli.execute && deployment.wallet_create_needed() {
        run_wallet_create(&client, owner, poll_config).await?;
    } else if deployment.wallet_create_needed() {
        println!(
            "Dry-run deploy step: would submit WALLET-CREATE first in live mode; expected wallet {}.",
            checksum(deposit_wallet)
        );
    }

    let prepared = prepare_approve_batch(
        &client,
        &owner_signer,
        contract_config,
        deposit_wallet,
        &approve,
    )
    .await?;
    print_prepared_batch(&prepared);

    if !cli.execute {
        println!("Result: DRY-RUN complete. Signed WALLET batch validated locally; submit skipped.");
        return Ok(());
    }

    let receipt = submit_wallet_batch_with_optional_nonce_refresh(
        &client,
        &owner_signer,
        contract_config,
        deposit_wallet,
        &approve,
        prepared,
    )
    .await?;
    ensure_submit_receipt_not_terminal("WALLET submit", &receipt.state)?;
    println!(
        "WALLET submit accepted: transaction_id={}, initial_state={}",
        public_token_summary(&receipt.transaction_id),
        receipt.state.label()
    );

    let confirmed = client
        .poll_transaction_for_owner_with_config(owner, &receipt.transaction_id, poll_config)
        .await
        .map_err(|error| classify_poll_error("WALLET poll", error))?;
    ensure_confirmed("WALLET poll", &confirmed.state)?;
    println!(
        "Result: CONFIRMED. transaction_id={}, transaction_hash={}",
        public_token_summary(&confirmed.transaction_id),
        confirmed
            .transaction_hash
            .as_deref()
            .map(public_token_summary)
            .unwrap_or_else(|| "<absent>".to_string())
    );

    Ok(())
}

#[derive(Debug)]
struct Cli {
    execute: bool,
    network: String,
}

enum CliAction {
    Run(Cli),
    Help,
}

fn parse_cli() -> Result<CliAction> {
    let mut execute = false;
    let mut network = DEFAULT_NETWORK.to_string();
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--execute" => execute = true,
            "--network" => {
                network = args
                    .next()
                    .ok_or_else(|| anyhow!("--network requires a value"))?;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(CliAction::Help);
            }
            _ if arg.starts_with("--network=") => {
                network = arg["--network=".len()..].to_string();
            }
            _ => bail!("unknown argument {arg}; use --help for usage"),
        }
    }

    Ok(CliAction::Run(Cli { execute, network }))
}

fn print_usage() {
    println!("Deposit-wallet Amoy ERC20 approve orchestrator");
    println!("Usage: cargo run --example deposit_wallet_live -- --network amoy [--execute]");
    println!("Default mode is dry-run and never submits POST /submit.");
    println!("Live mode requires --execute and {LIVE_GATE_ENV}=1.");
}

fn validate_network(network: &str) -> Result<()> {
    if network.eq_ignore_ascii_case("amoy") {
        return Ok(());
    }
    bail!("network {network:?} is refused; this example is Amoy-only on chain {AMOY_CHAIN_ID}")
}

fn print_header(cli: &Cli) {
    println!("=== Deposit Wallet Live Orchestrator Example ===");
    println!("Network: {}", cli.network.to_ascii_lowercase());
    println!("Chain ID: {AMOY_CHAIN_ID}");
    println!("Mode: {}", if cli.execute { "LIVE" } else { "DRY RUN" });
}

fn enforce_live_gate() -> Result<()> {
    match env::var(LIVE_GATE_ENV) {
        Ok(value) if value == "1" => Ok(()),
        _ => {
            println!("Live gate: REFUSED before reading secrets or sending HTTP requests.");
            println!("Reason: --execute also requires {LIVE_GATE_ENV}=1.");
            bail!("live execution refused by dual gate")
        }
    }
}

fn load_relayer_url() -> String {
    env::var(AMOY_RELAYER_URL_ENV).unwrap_or_else(|_| DEFAULT_AMOY_RELAYER_URL.to_string())
}

fn enforce_live_relayer_url(relayer_url: &str) -> Result<()> {
    if relayer_url == DEFAULT_AMOY_RELAYER_URL {
        return Ok(());
    }
    bail!(
        "live execution refused: {AMOY_RELAYER_URL_ENV} must exactly equal {DEFAULT_AMOY_RELAYER_URL}"
    )
}

fn validate_optional_owner_address(derived_owner: Address) -> Result<()> {
    let Some(explicit_owner) = optional_address_env(OWNER_ADDRESS_ENV)? else {
        return Ok(());
    };

    if explicit_owner != derived_owner {
        bail!(
            "owner mismatch: {OWNER_ADDRESS_ENV} does not match the address derived from {POLYMARKET_OWNER_PRIVATE_KEY_ENV}"
        );
    }
    Ok(())
}

fn load_relayer_auth() -> Result<RelayerKeyAuth> {
    let api_key = env::var(RELAYER_API_KEY_ENV)
        .map_err(|_| anyhow!("missing {RELAYER_API_KEY_ENV}; dry-run may read /deployed and /nonce"))?;
    let api_key_address = required_address_env(RELAYER_API_KEY_ADDRESS_ENV)?;
    RelayerKeyAuth::new(api_key, api_key_address).map_err(Into::into)
}

fn load_approve_config() -> Result<ApproveConfig> {
    Ok(ApproveConfig {
        token: required_address_env(APPROVE_TOKEN_ENV)?,
        spender: required_address_env(APPROVE_SPENDER_ENV)?,
        amount: match env::var(APPROVE_AMOUNT_ENV) {
            Ok(raw) => parse_u256_decimal(&raw, APPROVE_AMOUNT_ENV)?,
            Err(_) => max_u256(),
        },
    })
}

fn required_address_env(name: &str) -> Result<Address> {
    let raw = env::var(name).map_err(|_| anyhow!("missing {name}"))?;
    parse_address(&raw, name)
}

fn optional_address_env(name: &str) -> Result<Option<Address>> {
    match env::var(name) {
        Ok(raw) if raw.trim().is_empty() => Ok(None),
        Ok(raw) => parse_address(&raw, name).map(Some),
        Err(_) => Ok(None),
    }
}

fn parse_address(raw: &str, name: &str) -> Result<Address> {
    let address: Address = raw
        .parse()
        .map_err(|_| anyhow!("{name} must be a 0x-prefixed 20-byte address"))?;
    if address == Address::zero() {
        bail!("{name} must not be the zero address");
    }
    Ok(address)
}

fn parse_u256_decimal(raw: &str, name: &str) -> Result<U256> {
    U256::from_dec_str(raw.trim()).map_err(|_| anyhow!("{name} must be a decimal uint256 value"))
}

fn max_u256() -> U256 {
    U256::from_big_endian(&[0xff; 32])
}

#[derive(Clone, Copy)]
struct ApproveConfig {
    token: Address,
    spender: Address,
    amount: U256,
}

struct PreparedApproveBatch {
    signed: SignedDepositWalletBatch,
    nonce: U256,
    deadline: U256,
    call: DepositWalletCall,
}

async fn prepare_approve_batch(
    client: &DepositWalletRelayerClient,
    owner_signer: &DepositWalletOwnerSigner,
    contract_config: DepositWalletContractConfig,
    deposit_wallet: Address,
    approve: &ApproveConfig,
) -> Result<PreparedApproveBatch> {
    println!("Fetching fresh WALLET nonce immediately before signing...");
    let nonce = client
        .get_wallet_nonce(owner_signer.owner_address())
        .await
        .map_err(|error| classify_read_error("fresh WALLET nonce", error))?;
    let deadline = fresh_deadline()?;
    let call = build_erc20_approve_call(approve.token, approve.spender, approve.amount)?;
    let signed = owner_signer.sign_deposit_wallet_batch(
        deposit_wallet,
        AMOY_CHAIN_ID,
        nonce,
        deadline,
        vec![call.clone()],
    )?;
    let _validated_request =
        build_deposit_wallet_batch_request_from_signed(signed.clone(), contract_config)?;

    Ok(PreparedApproveBatch {
        signed,
        nonce,
        deadline,
        call,
    })
}

fn fresh_deadline() -> Result<U256> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| anyhow!("system clock is before UNIX_EPOCH"))?;
    let deadline = now
        .checked_add(DEADLINE_VALID_FOR)
        .ok_or_else(|| anyhow!("deadline overflow"))?;
    Ok(U256::from(deadline.as_secs()))
}

async fn run_wallet_create(
    client: &DepositWalletRelayerClient,
    owner: Address,
    poll_config: DepositWalletPollingConfig,
) -> Result<()> {
    println!("Live deploy step: submitting WALLET-CREATE before signing WALLET batch...");
    let receipt = client
        .submit_wallet_create(owner)
        .await
        .map_err(|error| classify_submit_error("WALLET-CREATE submit", error))?;
    ensure_submit_receipt_not_terminal("WALLET-CREATE submit", &receipt.state)?;
    println!(
        "WALLET-CREATE accepted: transaction_id={}, initial_state={}",
        public_token_summary(&receipt.transaction_id),
        receipt.state.label()
    );

    let confirmed = client
        .poll_transaction_for_owner_with_config(owner, &receipt.transaction_id, poll_config)
        .await
        .map_err(|error| classify_poll_error("WALLET-CREATE poll", error))?;
    ensure_confirmed("WALLET-CREATE poll", &confirmed.state)?;
    println!(
        "WALLET-CREATE confirmed: transaction_id={}, transaction_hash={}",
        public_token_summary(&confirmed.transaction_id),
        confirmed
            .transaction_hash
            .as_deref()
            .map(public_token_summary)
            .unwrap_or_else(|| "<absent>".to_string())
    );
    Ok(())
}

async fn submit_wallet_batch_with_optional_nonce_refresh(
    client: &DepositWalletRelayerClient,
    owner_signer: &DepositWalletOwnerSigner,
    contract_config: DepositWalletContractConfig,
    deposit_wallet: Address,
    approve: &ApproveConfig,
    prepared: PreparedApproveBatch,
) -> Result<polymarket_relayer::deposit_wallet::DepositWalletTransactionReceipt> {
    match client.submit_signed_wallet_batch(prepared.signed).await {
        Ok(receipt) => Ok(receipt),
        Err(error) if is_clear_stale_nonce_before_acceptance(&error) => {
            println!("Submit rejected with a clear stale nonce before acceptance; refreshing nonce once.");
            let refreshed = prepare_approve_batch(
                client,
                owner_signer,
                contract_config,
                deposit_wallet,
                approve,
            )
            .await?;
            client
                .submit_signed_wallet_batch(refreshed.signed)
                .await
                .map_err(|error| classify_submit_error("WALLET submit after nonce refresh", error))
        }
        Err(error) => Err(classify_submit_error("WALLET submit", error)),
    }
}

fn ensure_submit_receipt_not_terminal(context: &str, state: &RelayerTransactionState) -> Result<()> {
    match state {
        RelayerTransactionState::Failed => bail!("{context}: FAILED at submit acceptance"),
        RelayerTransactionState::Invalid => bail!("{context}: INVALID at submit acceptance"),
        RelayerTransactionState::Unknown(_) => {
            bail!("{context}: INCONCLUSIVE unknown submit state")
        }
        RelayerTransactionState::New
        | RelayerTransactionState::Executed
        | RelayerTransactionState::Mined
        | RelayerTransactionState::Confirmed => Ok(()),
    }
}

fn ensure_confirmed(context: &str, state: &RelayerTransactionState) -> Result<()> {
    if state.is_success() {
        Ok(())
    } else {
        bail!("{context}: INCONCLUSIVE expected STATE_CONFIRMED, got {}", state.label())
    }
}

fn classify_read_error(context: &str, error: RelayerError) -> anyhow::Error {
    if error.is_deposit_wallet_reconciliation_required() {
        anyhow!("{context}: INCONCLUSIVE: {error}")
    } else {
        anyhow!("{context}: FAILED: {error}")
    }
}

fn classify_submit_error(context: &str, error: RelayerError) -> anyhow::Error {
    anyhow!("{context}: INCONCLUSIVE before confirmed transaction acceptance: {error}")
}

fn classify_poll_error(context: &str, error: RelayerError) -> anyhow::Error {
    match error {
        RelayerError::TransactionFailed(_) | RelayerError::TransactionInvalid(_) => {
            anyhow!("{context}: FAILED: {error}")
        }
        RelayerError::Timeout => anyhow!("{context}: INCONCLUSIVE: polling timed out"),
        other if other.is_deposit_wallet_reconciliation_required() => {
            anyhow!("{context}: INCONCLUSIVE: {other}")
        }
        other => anyhow!("{context}: INCONCLUSIVE: {other}"),
    }
}

fn is_clear_stale_nonce_before_acceptance(error: &RelayerError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("nonce")
        && ["stale", "already used", "used nonce", "nonce too low", "invalid nonce"]
            .iter()
            .any(|needle| message.contains(needle))
}

fn print_public_config(
    owner: Address,
    relayer_url: &str,
    approve: &ApproveConfig,
    poll_config: DepositWalletPollingConfig,
) {
    println!("Relayer URL: {relayer_url}");
    println!("Owner: {}", checksum(owner));
    println!("Approve token: {}", checksum(approve.token));
    println!("Approve spender: {}", checksum(approve.spender));
    println!("Approve amount: {}", approve.amount);
    println!(
        "Deadline freshness: {} seconds from signing time",
        DEADLINE_VALID_FOR.as_secs()
    );
    println!(
        "Polling: initial={}ms max={}ms timeout={}s; success requires STATE_CONFIRMED",
        poll_config.initial_backoff.as_millis(),
        poll_config.max_backoff.as_millis(),
        poll_config.timeout.as_secs()
    );
}

fn print_deployment(deployment: &DepositWalletDeployment) {
    match deployment {
        DepositWalletDeployment::Deployed { deposit_wallet, .. } => {
            println!("Wallet discovery: Deployed at {}", checksum(*deposit_wallet));
        }
        DepositWalletDeployment::CreateNeeded {
            expected_deposit_wallet,
            ..
        } => {
            println!(
                "Wallet discovery: CreateNeeded; expected wallet {}",
                checksum(*expected_deposit_wallet)
            );
        }
        _ => {
            println!(
                "Wallet discovery: unrecognized deployment variant; reported wallet {}",
                checksum(deployment.deposit_wallet())
            );
        }
    }
}

fn print_prepared_batch(prepared: &PreparedApproveBatch) {
    println!("Prepared WALLET batch: calls=1, signature=<redacted>, signed_body=<redacted>");
    println!("Batch nonce: {}", prepared.nonce);
    println!("Batch deadline unix: {}", prepared.deadline);
    println!("EIP-712 verifyingContract: {}", checksum(prepared.signed.deposit_wallet()));
    println!("Call target: {}", checksum(prepared.call.target));
    println!("Call value: {}", prepared.call.value);
    println!("Call data: selector=0x095ea7b3 bytes={}", prepared.call.data.len());
}

fn checksum(address: Address) -> String {
    to_checksum(&address, None)
}

fn public_token_summary(value: &str) -> String {
    let hash = hex::encode(keccak256(value.as_bytes()));
    format!("sha3:0x{}...{}", &hash[..8], &hash[56..])
}
