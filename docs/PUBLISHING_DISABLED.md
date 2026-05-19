# PUBLISHING_DISABLED.md

## Decision

This fork is not published to crates.io.

It is consumed as:

```toml
# development
rs-builder-relayer-client = { path = "../rs-builder-relayer-client" }

# production after review
rs-builder-relayer-client = {
  git = "ssh://git@github.com/DongwonTTuna/rs-builder-relayer-client.git",
  rev = "<audited_commit_sha>"
}
```

## Reason

- The crate is venue-facing critical infrastructure.
- Public publishing can imply unsupported official SDK status.
- Production needs explicit audit/provenance and commit pinning.
- The fork is incomplete until deposit-wallet `WALLET-CREATE`, `WALLET`, EIP-712, polling, and pUSD/CTF calldata tests pass.

## Forbidden

```bash
cargo publish
```

```toml
rs-builder-relayer-client = "0.1"
rs-builder-relayer-client = { git = "...", branch = "main" }
rs-builder-relayer-client = "*"
rs-builder-relayer-client = { git = "ssh://git@github.com/OrderBookTrade/rs-builder-relayer-client.git", rev = "..." }
```

## Re-Approval Requirement

Publishing can only be reconsidered after:

```text
1. explicit operator approval;
2. license review;
3. supply-chain review;
4. public API review;
5. official-doc parity review;
6. all fork acceptance tests pass;
7. clear non-official SDK disclaimer in README and crate metadata.
```
