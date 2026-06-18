use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

const SECRET_ENV_NAMES: &[&str] = &[
    "POLYMARKET_RELAYER_ALLOW_LIVE_AMOY",
    "POLYMARKET_OWNER_PRIVATE_KEY",
    "POLYMARKET_OWNER_ADDRESS",
    "POLYMARKET_RELAYER_API_KEY",
    "POLYMARKET_RELAYER_API_KEY_ADDRESS",
    "POLYMARKET_AMOY_RELAYER_URL",
    "POLYMARKET_AMOY_APPROVE_TOKEN",
    "POLYMARKET_AMOY_APPROVE_SPENDER",
    "POLYMARKET_AMOY_APPROVE_AMOUNT",
];

const SENTINEL_OWNER_SECRET: &str = "task14_should_not_load_owner_secret";
const SENTINEL_RELAYER_SECRET: &str = "task14_should_not_load_relayer_secret";

fn temp_dir(label: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rs_builder_relayer_{label}_{}_{}",
        std::process::id(),
        unique
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_sentinel_dotenv(dir: &Path) {
    fs::write(
        dir.join(".env"),
        format!(
            "POLYMARKET_RELAYER_ALLOW_LIVE_AMOY=1\nPOLYMARKET_OWNER_PRIVATE_KEY={SENTINEL_OWNER_SECRET}\nPOLYMARKET_RELAYER_API_KEY={SENTINEL_RELAYER_SECRET}\n"
        ),
    )
    .unwrap();
}

fn run_example(current_dir: &Path, args: &[&str]) -> Output {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mut command = Command::new(cargo);
    command
        .arg("run")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--example")
        .arg("deposit_wallet_live")
        .arg("--")
        .args(args)
        .current_dir(current_dir)
        .env("CARGO_TERM_COLOR", "never");
    for name in SECRET_ENV_NAMES {
        command.env_remove(name);
    }
    command.output().unwrap()
}

fn rendered_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_no_sentinel_secret(rendered: &str) {
    assert!(!rendered.contains(SENTINEL_OWNER_SECRET));
    assert!(!rendered.contains(SENTINEL_RELAYER_SECRET));
}

#[test]
fn deposit_wallet_live_example_fails_closed_before_dotenv_secret_loading() {
    let dir = temp_dir("live_gate");
    write_sentinel_dotenv(&dir);

    let output = run_example(&dir, &["--network", "amoy", "--execute"]);
    let rendered = rendered_output(&output);
    let _ = fs::remove_dir_all(&dir);

    assert!(!output.status.success(), "example unexpectedly succeeded: {rendered}");
    assert!(
        rendered.contains("Live gate: REFUSED before reading secrets or sending HTTP requests."),
        "missing fail-closed gate message: {rendered}"
    );
    assert!(
        rendered.contains("live execution refused by dual gate"),
        "missing dual-gate error: {rendered}"
    );
    assert_no_sentinel_secret(&rendered);
    assert!(!rendered.contains("Relayer URL:"));
}

#[test]
fn deposit_wallet_live_example_refuses_off_amoy_before_dotenv_secret_loading() {
    let dir = temp_dir("off_amoy");
    write_sentinel_dotenv(&dir);

    let output = run_example(&dir, &["--network", "polygon"]);
    let rendered = rendered_output(&output);
    let _ = fs::remove_dir_all(&dir);

    assert!(!output.status.success(), "example unexpectedly succeeded: {rendered}");
    assert!(
        rendered.contains("this example is Amoy-only on chain 80002"),
        "missing Amoy-only refusal: {rendered}"
    );
    assert_no_sentinel_secret(&rendered);
    assert!(!rendered.contains("Dry-run safety:"));
    assert!(!rendered.contains("Relayer URL:"));
}
