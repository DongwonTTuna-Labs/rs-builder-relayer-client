# deposit-wallet-relayer-client Specification

## ADDED Requirements

<!-- GROUP-E: endpoint/transport security and credential redaction -->

### Requirement: Group E - Typed production relayer URL boundary

The deposit-wallet relayer client MUST accept only a typed `DepositWalletRelayerUrl` for production endpoints, and that URL MUST be the HTTPS origin `https://relayer-v2.polymarket.com/` with no userinfo, host exactly `relayer-v2.polymarket.com`, default port 443 only, path exactly `/`, and no query or fragment. Derived request endpoints MUST keep the same origin while setting only the relayer path/query needed by the request.

#### Scenario: Production relayer URL is accepted as a typed endpoint

- **Given** the raw URL `https://relayer-v2.polymarket.com` and the Polygon deposit-wallet contract config
- **When** the caller parses the URL and constructs a deposit-wallet relayer client
- **Then** the URL is accepted as a production relayer URL
- **And** request endpoints may set paths such as `/submit`, `/nonce`, or `/transaction` without changing the origin

#### Scenario: Non-allowlisted or decorated production URLs are rejected

- **Given** a raw relayer URL using `http`, userinfo, a host other than exactly `relayer-v2.polymarket.com`, a prefix or suffix lookalike host, a non-default HTTPS port, a path other than `/`, a query, or a fragment
- **When** the caller parses the URL as a production `DepositWalletRelayerUrl`
- **Then** the client MUST reject it as an invalid relayer URL before any HTTP request can be built

#### Scenario: Production endpoint rejects non-Polygon contract config

- **Given** the production relayer URL and a deposit-wallet contract config for a non-Polygon chain
- **When** the caller constructs a deposit-wallet relayer client
- **Then** the client MUST reject the endpoint/config pairing before it can send authenticated traffic

### Requirement: Group E - Redirect-disabled HTTPS transport policy

The deposit-wallet relayer client MUST build its HTTP transport with redirects disabled and a 30 second timeout. A redirect response from the relayer MUST be treated as a relayer API error, not followed to another origin or path.

#### Scenario: Client transport is created with redirect denial and timeout

- **Given** a typed relayer URL, relayer-key auth, and deposit-wallet contract config
- **When** a deposit-wallet relayer client is constructed
- **Then** the underlying HTTP client is configured with redirect policy `none`
- **And** the request timeout is 30 seconds

#### Scenario: Redirect response is not followed with credentials

- **Given** a relayer response with HTTP `302 Found` and a `Location` pointing to another endpoint
- **When** the client sends an authenticated relayer request
- **Then** the client MUST return an API error with status `302`
- **And** the client MUST NOT follow the redirect or send relayer credentials to the redirect target

### Requirement: Group E - Relayer key auth validation and credential redaction

`RelayerKeyAuth` MUST store the API key as secret material, validate key strings before construction, send only the implemented relayer-key headers, mark credential headers sensitive, and redact credential values from debug output. This client MUST NOT define a `Display` contract for `RelayerKeyAuth`, and MUST NOT claim bearer-token or HMAC authentication for the deposit-wallet relayer path.

#### Scenario: Valid relayer-key auth creates sensitive headers

- **Given** a non-empty relayer API key within the supported size limit and a relayer API key address
- **When** the client builds authenticated request headers
- **Then** it sends `RELAYER_API_KEY` with the API key value
- **And** it sends `RELAYER_API_KEY_ADDRESS` with the checksummed relayer API key address
- **And** both credential header values are marked sensitive

#### Scenario: Invalid API keys are rejected before header construction

- **Given** an empty API key, a whitespace-only key, a key containing whitespace or control characters, a key longer than 4096 bytes, or a key that cannot become an HTTP header value
- **When** the caller constructs `RelayerKeyAuth`
- **Then** construction MUST fail with an auth error
- **And** no relayer credential headers are produced

#### Scenario: Debug output redacts raw credentials and unsupported auth schemes remain unsupported

- **Given** a `RelayerKeyAuth` and a client containing it
- **When** debug output is formatted
- **Then** the raw API key and raw API key address MUST NOT appear
- **And** callers MUST NOT rely on `Display`, bearer-token, or HMAC authentication behavior because the implemented auth path exposes only the two relayer-key headers

### Requirement: Group E - Relayer auth identity remains independent from owner identity

The relayer API key identity MUST be modeled separately from the deposit-wallet owner identity. Authenticated requests MUST use the `RelayerKeyAuth` API key address for `RELAYER_API_KEY_ADDRESS`, even when the request path or body is scoped to a different owner.

#### Scenario: Auth headers use relayer API key address while request uses owner

- **Given** a relayer API key address and a different deposit-wallet owner address
- **When** the client sends a nonce, submit, or transaction request for the owner
- **Then** the relayer auth header MUST contain the relayer API key address
- **And** the owner address MUST remain the owner-specific request field, query value, or response validation subject

#### Scenario: Owner address is not allowed to replace relayer auth identity

- **Given** relayer auth identity and deposit-wallet owner identity are not equal
- **When** the client builds authenticated headers for an owner-scoped request
- **Then** the client MUST NOT overwrite `RELAYER_API_KEY_ADDRESS` with the owner address
- **And** it MUST NOT require auth identity and owner identity to be equal as a credential-validation shortcut

### Requirement: Group E - Transport errors, Retry-After, body caps, and URL stripping

The transport layer MUST classify every non-2xx response as a relayer API error, preserve `Retry-After` timing for 429 responses, cap success and transaction response bodies, drain error bodies only within the configured cap, and strip request URLs from HTTP transport errors.

#### Scenario: Successful in-cap response bodies are read

- **Given** a 2xx relayer response whose body is within the regular success cap or the transaction-read success cap
- **When** the client reads the response body
- **Then** the body may be returned for the caller's parser to process
- **And** the client MUST NOT classify the response as an API error solely because transport body limiting was applied

#### Scenario: Non-2xx response becomes an API error with retry guidance when present

- **Given** a relayer response with a non-2xx status
- **When** the client receives the response
- **Then** it MUST return a `RelayerError::Api` containing the HTTP status
- **And** for HTTP `429 Too Many Requests`, it MUST preserve a numeric or HTTP-date `Retry-After` value as a retry-after seconds summary when the header can be parsed

#### Scenario: Success body limits are enforced for regular and transaction reads

- **Given** a regular success response larger than 64 KiB or a transaction-read success response larger than 256 KiB
- **When** the client reads the response body
- **Then** it MUST reject the response as exceeding the maximum body size
- **And** it MUST enforce the limit both when `Content-Length` is too large and when streamed chunks exceed the cap

#### Scenario: Error body drain is bounded and transport errors strip URLs

- **Given** a non-2xx response with a large error body or a transport failure containing request URL context
- **When** the client handles the response or transport error
- **Then** error body draining MUST be bounded to the configured error-drain cap
- **And** the returned HTTP transport error MUST NOT expose the request URL, local endpoint, or transaction identifier from the failed URL

<!-- GROUP-A: mutation gate default-deny and permit -->

### Requirement: Group A - Default-deny mutation gate and action surface

The deposit-wallet relayer client MUST default mutation-related public APIs to `DepositWalletMutationGate::Deny`. Without an explicit `Permit`, `submit_wallet_create`, `sign_and_submit_wallet_batch_with_nonce_lease`, and nonce-lease reads MUST be blocked before HTTP request construction, endpoint dispatch, or relayer auth header construction with the message `explicit deposit-wallet mutation permit required` wrapped by the stable `Deposit-wallet mutation blocked:` error prefix. The implemented action set is exactly `WalletNonceRead`, `WalletCreate`, `WalletBatch`, `OwnerRecoveryPoll`, and `ManualReconciliation`; `OwnerRecoveryPoll` exists as a variant but is not currently used as public enforcement. The gate has no read-only, dry-run, or cancel-only mode.

#### Scenario: Explicit permits allow the implemented test-loopback mutation actions

- **Given** a test-loopback deposit-wallet relayer client and owner-scoped permit evidence for `WalletNonceRead`, `WalletCreate`, or `WalletBatch`
- **When** the caller invokes the matching nonce-lease read, WALLET-CREATE submit, or WALLET batch submit path
- **Then** the mutation gate may pass the matching action check
- **And** later request construction or owner-state checks proceed only after the permit has been validated

#### Scenario: Default Deny blocks nonce-lease reads before HTTP or auth work

- **Given** a deposit-wallet relayer client and `DepositWalletMutationGate::Deny`
- **When** the caller requests a WALLET nonce lease for an owner
- **Then** the client MUST return a mutation-blocked error containing `explicit deposit-wallet mutation permit required`
- **And** it MUST NOT construct relayer auth headers or send a `/nonce` HTTP request

#### Scenario: Default Deny blocks WALLET-CREATE before HTTP or auth work

- **Given** a deposit-wallet relayer client and `DepositWalletMutationGate::Deny`
- **When** the caller submits WALLET-CREATE for an owner
- **Then** the client MUST return a mutation-blocked error containing `explicit deposit-wallet mutation permit required`
- **And** it MUST NOT construct relayer auth headers or send a `/submit` HTTP request

#### Scenario: Default Deny blocks WALLET batch submit after an already-permitted nonce lease

- **Given** a current WALLET nonce lease obtained under a valid `WalletNonceRead` permit
- **When** the caller attempts `sign_and_submit_wallet_batch_with_nonce_lease` with `DepositWalletMutationGate::Deny`
- **Then** the client MUST return a mutation-blocked error containing `explicit deposit-wallet mutation permit required`
- **And** it MUST NOT proceed to WALLET `/submit` HTTP work for that batch

### Requirement: Group A - Permit owner and exact scope matching

A mutation permit MUST match the requested owner and the exact client mutation scope before any protected mutation action continues. Scope matching MUST include chain id, factory address, implementation address, mutation environment, and action. A permit for one owner, action, environment, chain, factory, or implementation MUST NOT authorize another owner or scope.

#### Scenario: Matching owner and scope permit passes the requested action

- **Given** a permit whose owner matches the request owner
- **And** the permit evidence scope matches the client's chain id, factory, implementation, environment, and requested action
- **When** the client validates the permit for that owner and action
- **Then** owner validation, scope validation, and freshness validation may pass
- **And** the protected mutation path may continue to later owner-state or request-building checks

#### Scenario: Owner mismatch is rejected

- **Given** a permit created for owner A
- **When** the client validates that permit for owner B
- **Then** the client MUST return a mutation-blocked error
- **And** the rejected permit MUST NOT authorize the requested mutation action

#### Scenario: Scope, action, or environment mismatch is rejected

- **Given** a permit whose evidence scope differs from the expected client scope by action, environment, chain id, factory, or implementation
- **When** the client validates the permit for the requested mutation action
- **Then** the client MUST return a mutation-blocked error
- **And** no protected mutation work may continue under the mismatched scope

### Requirement: Group A - Permit freshness and evidence lifetime

A mutation permit MUST carry a non-empty reason and owner-serialization evidence that is fresh at validation time. Evidence MUST NOT be acquired in the future, MUST NOT be older than the 300 second maximum owner-serialization lease window, and MUST NOT be expired at the validation timestamp. Evidence construction MUST reject missing issuer, missing lease id, non-increasing expiry, and lease durations longer than 300 seconds.

#### Scenario: Fresh evidence within the 300 second lease window is accepted

- **Given** owner-serialization evidence with a non-empty issuer, non-empty lease id, acquisition time no later than validation time, expiry after validation time, and acquisition age at or below 300 seconds
- **And** a permit with a non-empty reason
- **When** the client validates the permit for the matching owner and scope
- **Then** the freshness check may pass
- **And** the mutation action may continue to later owner-state or request-building checks

#### Scenario: Empty permit reason is rejected

- **Given** otherwise valid owner-serialization evidence
- **When** the caller constructs or validates a permit with an empty or whitespace-only reason
- **Then** the client MUST return a mutation-blocked error
- **And** the permit MUST NOT authorize any mutation action

#### Scenario: Future or stale acquisition evidence is rejected

- **Given** owner-serialization evidence whose acquisition timestamp is after the validation time or more than 300 seconds before the validation time
- **When** the client validates the permit freshness
- **Then** the client MUST return a mutation-blocked error for future or stale evidence
- **And** no protected mutation work may continue

#### Scenario: Expired or oversized lease evidence is rejected

- **Given** owner-serialization evidence whose expiry is at or before validation time, whose expiry is not after acquisition, or whose lease duration exceeds 300 seconds
- **When** the evidence is constructed or the permit is validated
- **Then** the client MUST reject the evidence or return a mutation-blocked error
- **And** the permit MUST NOT authorize any mutation action

### Requirement: Group A - Production permit construction remains unavailable

Public construction of production mutation permits MUST be blocked in this PR. `DepositWalletMutationPermit::from_owner_serialization_evidence` MUST reject evidence whose scope environment is `Production` for `WalletNonceRead`, `WalletCreate`, `WalletBatch`, `OwnerRecoveryPoll`, or `ManualReconciliation`. `TestLoopback` permit construction exists only through cfg(test) loopback paths and MUST NOT be described as a public production permit issuance mechanism.

#### Scenario: Public production permit construction is refused for every mutation action

- **Given** a production relayer URL and production mutation scope for `WalletNonceRead`, `WalletCreate`, `WalletBatch`, `OwnerRecoveryPoll`, or `ManualReconciliation`
- **When** the caller passes matching owner-serialization evidence to `DepositWalletMutationPermit::from_owner_serialization_evidence`
- **Then** public permit construction MUST fail with a mutation-blocked error
- **And** the error MUST explain that durable owner state and a crate-owned trusted capability are required before public production permits are enabled

#### Scenario: TestLoopback permits are limited to test-only paths

- **Given** a cfg(test) loopback relayer URL and matching `TestLoopback` mutation scope
- **When** unit tests construct owner-serialization evidence and a permit for a matching owner/action
- **Then** the permit may be constructed for test-loopback coverage
- **And** that path MUST NOT be treated as live production permit issuance

### Requirement: Group A - Stable mutation-blocked error prefix and classifier

Every mutation-gate denial created through `RelayerError::mutation_blocked` MUST be represented as `RelayerError::Other` with the stable string prefix `Deposit-wallet mutation blocked:`. Consumers MUST be able to classify this error through `is_deposit_wallet_mutation_blocked` instead of parsing the prefix themselves.

#### Scenario: Mutation blocked errors keep a stable prefix and classifier

- **Given** a mutation gate denial or permit validation failure
- **When** the client returns the error through `RelayerError::mutation_blocked`
- **Then** the displayed error MUST start with `Deposit-wallet mutation blocked:`
- **And** `is_deposit_wallet_mutation_blocked` MUST return true for that error

#### Scenario: Non-mutation errors are not classified as mutation blocked

- **Given** a relayer error whose message does not use the mutation-blocked prefix
- **When** the caller checks `is_deposit_wallet_mutation_blocked`
- **Then** the classifier MUST NOT return true solely because the error is another `RelayerError::Other` value

<!-- GROUP-C: submit flow ordering and client construction -->

### Requirement: Group C - WALLET-CREATE submit ordering and body

`submit_wallet_create(owner, gate)` MUST execute the WALLET-CREATE submit flow in this order: validate the `WalletCreate` mutation gate, confirm the owner has no in-flight submit, ambiguous submit, or active nonce-read block, build and serialize the WALLET-CREATE request body, reserve the owner submit with the body payload hash, build relayer auth headers, derive the `/submit` endpoint, arm the reservation for ambiguous-on-drop, and send the POST request. Gate and owner-block checks MUST happen before relayer auth header construction or HTTP POST work. The request body MUST use the canonical WALLET-CREATE shape with `type`, `from`, and `to` fields.

#### Scenario: Permitted unblocked owner submits canonical WALLET-CREATE body

- **Given** an owner with a valid `WalletCreate` permit and no local owner mutation block
- **When** the caller invokes `submit_wallet_create(owner, gate)`
- **Then** the client MUST check the mutation gate before checking owner mutation state
- **And** it MUST build and serialize a WALLET-CREATE body whose `type` is `WALLET-CREATE`, whose `from` is the owner, and whose `to` is the configured factory
- **And** it MUST reserve the owner submit before constructing relayer auth headers
- **And** it MUST arm the reservation before sending `POST /submit`

#### Scenario: Gate denial stops WALLET-CREATE before auth or HTTP

- **Given** `DepositWalletMutationGate::Deny`
- **When** the caller invokes `submit_wallet_create(owner, gate)`
- **Then** the client MUST return a mutation-blocked error for the missing explicit permit
- **And** it MUST NOT build relayer auth headers, reserve owner submit state, derive `/submit`, or send HTTP traffic

#### Scenario: Existing owner mutation state blocks WALLET-CREATE before body or POST

- **Given** the owner already has an in-flight submit, an ambiguous submit, or an active nonce-read reservation
- **When** the caller invokes `submit_wallet_create(owner, gate)` with an otherwise valid permit
- **Then** the client MUST return the owner-state blocking error before WALLET-CREATE body construction that can proceed to reservation
- **And** it MUST NOT build relayer auth headers or send another `POST /submit` for that owner

#### Scenario: Pre-acceptance failures do not leave a live WALLET-CREATE reservation

- **Given** WALLET-CREATE body serialization fails before reservation, relayer auth header construction fails after reservation, or the relayer returns a pre-acceptance API status that the implementation treats as not accepted
- **When** `submit_wallet_create(owner, gate)` returns that error
- **Then** serialization failure MUST leave no owner submit reservation to clear because reservation has not opened yet
- **And** auth-header failure or supported pre-acceptance API failure MUST clear the owner submit reservation instead of leaving an ambiguous owner block

### Requirement: Group C - WALLET nonce lease read ordering

`get_wallet_nonce_with_lease(owner, gate)` MUST execute the leased WALLET nonce read in this order: validate the `WalletNonceRead` mutation gate, block production nonce-lease reads for this PR, derive the lease expiry from the permit, reserve the owner nonce-read slot, send `GET /nonce?address=<checksum(owner)>&type=WALLET`, parse the nonce response, and return a `DepositWalletNonceLease` containing the owner, nonce, expiry, and live owner reservation. Gate checks and production-read blocking MUST happen before the reservation and before any HTTP/auth work.

#### Scenario: Permitted loopback nonce lease returns owner-bound lease

- **Given** a test-loopback client, a valid `WalletNonceRead` permit, and an owner without in-flight owner mutation state
- **When** the caller invokes `get_wallet_nonce_with_lease(owner, gate)`
- **Then** the client MUST check the gate before production-read blocking and owner reservation
- **And** it MUST reserve the owner nonce-read slot before sending the nonce request
- **And** it MUST send `GET /nonce` with query parameters `address=<checksum(owner)>` followed by `type=WALLET`
- **And** it MUST return a nonce lease bound to that owner and nonce, with the permit-derived expiry and the live reservation

#### Scenario: Gate denial or production lease read blocks before HTTP

- **Given** either `DepositWalletMutationGate::Deny` or a production relayer URL with a trusted permit token
- **When** the caller invokes `get_wallet_nonce_with_lease(owner, gate)`
- **Then** the client MUST return a mutation-blocked error before sending `GET /nonce`
- **And** production lease reads MUST remain disabled in this PR even if a test-only trusted production permit token is supplied

#### Scenario: Owner reservation blocks competing work and releases on nonce fetch failure

- **Given** the owner already has an active nonce-read reservation, in-flight submit, or ambiguous submit block
- **When** another nonce-lease read or owner mutation is attempted for the same owner
- **Then** the client MUST reject the competing work before an additional HTTP request for that owner
- **And** if nonce fetch fails after the nonce-read reservation is opened, the reservation MUST be released so a later nonce-lease read can retry

### Requirement: Group C - Lease-bound WALLET batch signing and submit ordering

`sign_and_submit_wallet_batch_with_nonce_lease(lease, gate, sign)` MUST submit a WALLET batch only through the nonce-lease flow. The method MUST validate the `WalletBatch` mutation gate before constructing the signing context, derive chain id from client config, derive the expected deposit wallet, compute the nonce-lease signing binding, pass a `DepositWalletNonceLeaseSigningContext` to the caller's signing closure, validate the `WalletBatch` gate a second time after signing, validate that the signed batch matches the current lease and client expectations, consume the unexpired nonce lease, promote the nonce reservation to an owner submit reservation, build and serialize the WALLET body, update the reservation payload hash, build relayer auth headers, and send `POST /submit`. Signing MUST be closure-injected through the nonce-lease context, not performed from a client-retained signer.

#### Scenario: Lease-bound signing context produces canonical WALLET POST body

- **Given** a current WALLET nonce lease and a valid `WalletBatch` permit for the lease owner
- **When** the caller invokes `sign_and_submit_wallet_batch_with_nonce_lease` and the closure validates a signed batch from the provided context
- **Then** the signing context MUST expose the owner, nonce owner, submit-from owner, nonce, derived deposit wallet, and config-derived chain id expected by the lease
- **And** the client MUST perform the second gate check after the closure returns the signed batch
- **And** it MUST consume the lease, promote it to an owner submit reservation, build the WALLET body, construct relayer auth headers, arm the reservation, and send `POST /submit`
- **And** the POST body MUST use the canonical WALLET shape with `type`, `from`, `to`, `nonce`, `signature`, and `depositWalletParams`

#### Scenario: Gate denial prevents WALLET signing or submit before auth

- **Given** a current WALLET nonce lease but `DepositWalletMutationGate::Deny` for the WALLET batch submit
- **When** the caller invokes `sign_and_submit_wallet_batch_with_nonce_lease`
- **Then** the first gate check MUST return a mutation-blocked error before the signing context is built
- **And** the client MUST NOT invoke the signing closure, build relayer auth headers, or send `POST /submit`

#### Scenario: Signed batch mismatch or validation failure stops before POST

- **Given** a nonce lease and a signed batch that is unbound, bound to an older lease, for a different owner, with a mismatched nonce owner, submit-from owner, chain id, deposit wallet, nonce, or expired deadline
- **When** the caller submits that signed batch through the nonce-lease flow
- **Then** the client MUST reject it with a signing, mutation-blocked, or reconciliation-required error according to the failed validation
- **And** it MUST NOT build relayer auth headers or send a WALLET `POST /submit` for that invalid batch
- **And** it MUST NOT create an ambiguous submit block from validation failures that occur before the POST boundary

#### Scenario: WALLET body, serialization, and auth failures clear promoted reservations where supported

- **Given** a valid nonce lease has been promoted to an owner submit reservation for WALLET batch submit
- **When** request building, body serialization, or relayer auth header construction fails before the POST boundary
- **Then** the client MUST clear the promoted owner submit reservation before returning the error
- **And** the owner MUST NOT remain blocked as ambiguous for a failure that occurred before relayer acceptance

### Requirement: Group C - Client construction and signer separation

`DepositWalletRelayerClient::new(base_url: DepositWalletRelayerUrl, auth: RelayerKeyAuth, config: DepositWalletContractConfig) -> Result<Self>` MUST construct the deposit-wallet relayer client from a typed URL, relayer-key auth, and deposit-wallet contract config. The constructor MUST validate the URL/config pairing, build the HTTP client with the implemented transport policy, store the URL/auth/config/mutation state/clock, derive chain id from config when building mutation scopes or WALLET signing expectations, and MUST NOT accept or retain an owner signer. Debug formatting MUST redact through the stored `RelayerKeyAuth`; WALLET signing is supplied later by the caller's nonce-lease signing closure.

#### Scenario: Constructor accepts typed production client inputs without owner signer

- **Given** the production relayer URL, relayer-key auth, and the Polygon deposit-wallet contract config
- **When** the caller invokes `DepositWalletRelayerClient::new(base_url, auth, config)`
- **Then** construction may return a client without any owner-signer argument
- **And** later mutation scopes and WALLET signing expectations MUST derive chain id from the stored config

#### Scenario: Constructor rejects unsupported production config before client use

- **Given** the production relayer URL and a non-Polygon deposit-wallet contract config
- **When** the caller invokes `DepositWalletRelayerClient::new(base_url, auth, config)`
- **Then** construction MUST fail before authenticated traffic can be sent
- **And** the failed constructor MUST NOT create an owner mutation state that can be used for submit flow work

#### Scenario: Debug redacts auth and signing remains closure-injected

- **Given** a constructed client containing relayer-key auth
- **When** debug output is formatted
- **Then** the raw relayer API key and raw relayer API key address MUST NOT appear because auth debug redaction is used
- **And** the client MUST NOT expose a constructor-owned signer; the owner signature for WALLET batch submit MUST enter only through the `sign_and_submit_wallet_batch_with_nonce_lease` closure

<!-- GROUP-D+G: nonce read, transaction read, and transaction state machine -->

### Requirement: Group D - WALLET nonce read request and decimal parser

The deposit-wallet relayer client MUST build internal WALLET nonce reads as `GET /nonce?address=<checksum(owner)>&type=WALLET`, with the `address` query pair appended before `type=WALLET`. The nonce response parser MUST accept a JSON object whose `nonce` value is either a JSON string containing decimal ASCII digits or a JSON number token containing decimal ASCII digits. The accepted nonce decimal MUST contain 1-78 ASCII digits and fit within the `U256` range. The parser MUST reject empty strings, non-digit strings, non-ASCII digits, signed values, fractional or exponent tokens, responses longer than 78 digits, values outside `U256`, missing nonce fields, non-object bodies, and malformed JSON.

#### Scenario: Nonce request uses the exact WALLET path and query order

- **Given** a test-loopback deposit-wallet relayer client, a valid `WalletNonceRead` permit, and an owner address
- **When** the client fetches a WALLET nonce for that owner
- **Then** it MUST send exactly `GET /nonce?address=<checksum(owner)>&type=WALLET`
- **And** the `address` query MUST appear before the `type=WALLET` query
- **And** the request body MUST be empty

#### Scenario: Decimal nonce strings and numeric tokens within U256 are accepted

- **Given** a nonce response object whose `nonce` is a JSON string such as `"31"` or the decimal string form of `U256::MAX`
- **And** a nonce response object whose `nonce` is a JSON numeric token such as `31`
- **When** the client parses the response
- **Then** it MUST return the corresponding `U256` nonce
- **And** it MUST treat the accepted representation as decimal, not hexadecimal or exponent notation

#### Scenario: Invalid nonce response forms are rejected

- **Given** a nonce response whose `nonce` value is empty, non-decimal, hexadecimal-looking, signed, fractional, exponent-form, non-ASCII, more than 78 digits, or numerically outside `U256`
- **Or** a response body with no `nonce` field, a null nonce, an array body, a top-level string, or malformed JSON
- **When** the client parses the WALLET nonce response
- **Then** parsing MUST fail
- **And** the client MUST NOT silently coerce the malformed value into a nonce

### Requirement: Group D - Single owner transaction lookup and production read block

`get_transaction_for_owner(owner, transaction_id)` MUST first block production hosts. On non-production hosts, it MUST perform one owner-scoped transaction read by validating the caller-supplied transaction id and then sending a single `GET /transaction?id=<id>` request with the transaction-read body limit. This operation is not a poll loop and MUST NOT be described as live production polling behavior in this PR. Production transaction reads MUST be blocked before the request is trusted as live behavior until official or recorded WALLET transaction response evidence has been reviewed. Transaction ids MUST be non-empty after trim, 1-128 bytes, unchanged by leading/trailing whitespace trim, and free of ASCII control characters; opaque non-control punctuation remains accepted.

#### Scenario: Test-loopback transaction read sends one GET by id

- **Given** a test-loopback relayer response for transaction id `tx-owner`
- **When** the caller invokes `get_transaction_for_owner(owner, "tx-owner")`
- **Then** the client MUST send exactly one `GET /transaction?id=tx-owner` request
- **And** the request body MUST be empty
- **And** relayer authentication headers MUST still use the relayer API key identity rather than the owner identity

#### Scenario: Production transaction read is blocked before live trust

- **Given** the production relayer URL and any owner transaction id
- **When** the caller invokes `get_transaction_for_owner(owner, transaction_id)`
- **Then** the client MUST return a deposit-wallet read-blocked error
- **And** it MUST NOT treat the transaction read as production-ready behavior in this PR

#### Scenario: Malformed transaction ids are rejected before HTTP

- **Given** a transaction id that is empty after trim, longer than 128 bytes, has leading or trailing whitespace, or contains ASCII control characters
- **When** the caller invokes `get_transaction_for_owner(owner, transaction_id)`
- **Then** the client MUST reject the id before sending `GET /transaction`
- **And** no HTTP request MUST be recorded for that invalid id

### Requirement: Group D - Transaction response proof and identifier validation

Transaction response parsing MUST validate the requested transaction id and deposit-wallet proof evidence before returning a receipt. A matching object response or selected array item MUST have a valid `transactionID` equal to the requested id, a `type` in `{WALLET, WALLET-CREATE}`, owner evidence present, `from` equal to owner evidence, `to` equal to the configured factory, and `proxyAddress` equal to the derived deposit wallet. `get_transaction_for_owner(owner, tx_id)` MUST additionally require the response owner evidence to match the requested owner. When `transactionHash` is present, it MUST validate to the canonical `0x` plus 64 hex character shape; malformed hashes MUST require reconciliation rather than being trusted.

#### Scenario: Proven WALLET transaction response returns owner and derived wallet evidence

- **Given** a transaction response or matching array item with the requested `transactionID`, `type=WALLET`, `owner`, `from`, `to`, `proxyAddress`, `STATE_CONFIRMED`, and a valid 66-character transaction hash
- **When** the client parses the transaction response for the requested owner
- **Then** it MUST accept the proof evidence
- **And** the returned receipt MUST carry the requested transaction id, owner evidence, and derived deposit-wallet address evidence

#### Scenario: WALLET-CREATE transaction type is accepted with the same proof boundary

- **Given** a transaction response with `type=WALLET-CREATE`, owner evidence, `from` equal to owner, `to` equal to the configured factory, and `proxyAddress` equal to the derived wallet
- **When** the client parses the response for the requested transaction id
- **Then** the transaction type proof MAY be accepted
- **And** the same owner, factory, proxy-address, id, and hash validation rules MUST still apply

#### Scenario: Missing or mismatched owner proof requires reconciliation

- **Given** a transaction response with missing owner evidence, owner evidence that does not match the requested owner, `from` that does not match owner evidence, `to` that does not match the configured factory, or `proxyAddress` that does not match the derived wallet
- **When** `get_transaction_for_owner(owner, tx_id)` parses the response
- **Then** the client MUST return a deposit-wallet reconciliation-required error
- **And** it MUST NOT return the receipt as trusted owner transaction evidence

#### Scenario: Malformed transaction id or hash evidence is not trusted

- **Given** a transaction response whose `transactionID` is missing, non-string, not equal to the requested id, duplicated in the response array, or invalid after transaction-id validation
- **Or** a response whose `transactionHash` is present but too short, too long, missing the `0x` canonical shape, or contains non-hex characters
- **When** the client parses the transaction response
- **Then** it MUST reject the response or require reconciliation according to the parser path
- **And** a confirmed response with malformed hash evidence MUST NOT be returned as success

### Requirement: Group D - Owner transaction state classification

After transaction response proof validation, owner transaction classification MUST return success only for `Confirmed` receipts that include a validated transaction hash. `Confirmed` without a hash MUST require manual reconciliation. `Invalid` MUST return `TransactionInvalid`; `Failed` MUST return `TransactionFailed`; `Unknown(raw)` MUST require reconciliation while preserving the raw state in the enum and summarizing the raw value in outward error text; and `New`, `Executed`, or `Mined` MUST be reported as `transaction_absent` pending/non-terminal states.

#### Scenario: Confirmed transaction with valid hash is the only success

- **Given** a proven owner transaction response whose state is `STATE_CONFIRMED` and whose `transactionHash` validates to canonical `0x` plus 64 hex characters
- **When** `get_transaction_for_owner(owner, tx_id)` classifies the receipt
- **Then** the client MAY return the receipt as success
- **And** the receipt MUST include the validated transaction hash

#### Scenario: Confirmed transaction without hash requires reconciliation

- **Given** a proven owner transaction response whose state is `STATE_CONFIRMED` but whose `transactionHash` is absent or empty after trim
- **When** the client classifies the receipt
- **Then** it MUST return a deposit-wallet reconciliation-required error
- **And** it MUST NOT report the confirmed response as successful owner transaction completion

#### Scenario: Invalid and Failed terminal states return terminal failure errors

- **Given** a proven owner transaction response whose state is `STATE_INVALID`
- **When** the client classifies the receipt
- **Then** it MUST return `TransactionInvalid`
- **And** if the state is `STATE_FAILED`, it MUST return `TransactionFailed`

#### Scenario: Unknown state requires reconciliation and redacts raw outward text

- **Given** a proven owner transaction response whose state is an unrecognized raw value such as `STATE_FUTURE`
- **When** the client parses and classifies the receipt
- **Then** the `RelayerTransactionState` MUST preserve the raw value as `Unknown(raw)`
- **And** classification MUST return reconciliation-required with an unknown-state summary rather than trusting the state as success or exposing the raw state verbatim in the error text

#### Scenario: New, Executed, and Mined remain pending transaction_absent states

- **Given** a proven owner transaction response whose state is `STATE_NEW`, `STATE_EXECUTED`, or `STATE_MINED`
- **When** the client classifies the receipt
- **Then** it MUST return a deposit-wallet `transaction_absent` error
- **And** callers MUST treat the transaction as pending/non-terminal rather than successful

### Requirement: Group G - RelayerTransactionState wire mapping and terminal policy

`RelayerTransactionState` MUST contain exactly the implemented states `New`, `Executed`, `Mined`, `Confirmed`, `Invalid`, `Failed`, and `Unknown(String)`. Parsing MUST uppercase recognized values, accept both prefixed `STATE_*` and unprefixed recognized labels, and preserve unrecognized raw input in `Unknown(raw)`. Serialization MUST emit `STATE_NEW`, `STATE_EXECUTED`, `STATE_MINED`, `STATE_CONFIRMED`, `STATE_INVALID`, and `STATE_FAILED` for known states, while serializing unknown states as their preserved raw value. The terminal set MUST be `{Confirmed, Invalid, Failed}`, and `is_success()` MUST be true only for `Confirmed`.

#### Scenario: Known wire states map to implemented enum variants

- **Given** raw state strings such as `STATE_NEW`, `STATE_EXECUTED`, `STATE_MINED`, `STATE_CONFIRMED`, `STATE_INVALID`, and `STATE_FAILED`
- **When** the client deserializes relayer transaction state
- **Then** it MUST map them to `New`, `Executed`, `Mined`, `Confirmed`, `Invalid`, and `Failed`
- **And** serializing those variants MUST produce the corresponding `STATE_*` wire strings

#### Scenario: Unknown wire state preserves raw input

- **Given** an unrecognized state string such as `STATE_FUTURE`
- **When** the client parses the state
- **Then** it MUST store the original raw string in `Unknown(raw)`
- **And** serializing that unknown state MUST use the preserved raw string rather than inventing a known state

#### Scenario: Terminal and success helpers classify only the implemented terminal set

- **Given** each implemented `RelayerTransactionState` value
- **When** callers check terminal and success helper behavior
- **Then** only `Confirmed`, `Invalid`, and `Failed` MUST be terminal
- **And** only `Confirmed` MUST be success
- **And** `New`, `Executed`, `Mined`, and `Unknown(raw)` MUST NOT be treated as successful terminal completion

<!-- GROUP-F: wire DTO serialization and EIP-712 batch signing -->

### Requirement: Group F - Deposit-wallet wire DTO constants and serde shapes

Deposit-wallet relayer wire DTOs MUST use only the implemented constants and serde shapes. `WALLET_CREATE_TRANSACTION_TYPE` MUST serialize as `WALLET-CREATE`, and `WALLET_TRANSACTION_TYPE` MUST serialize as `WALLET`. `DepositWalletCall` MUST serialize with camelCase keys `target`, `value`, and `data`, where `target` is a checksummed address string, `value` is a decimal string, and `data` is `0x`-prefixed lowercase hex. `DepositWalletCreateRequest` MUST serialize `type`, `from`, and `to`. `DepositWalletParams` MUST serialize camelCase `depositWallet`, `deadline`, and `calls`, where `deadline` is a decimal string. `DepositWalletBatchRequest` MUST serialize camelCase `type`, `from`, `to`, `nonce`, `signature`, and `depositWalletParams`, where `nonce` is a decimal string. `RelayerSubmitResponse` MUST deserialize exact relayer keys `transactionID`, `state`, and optional `transactionHash`; `transactionId` is not an accepted alias.

#### Scenario: WALLET-CREATE fixture uses the canonical submit body

- **Given** an owner address and the configured deposit-wallet factory
- **When** the client builds a WALLET-CREATE request body for `POST /submit`
- **Then** the serialized `type` MUST be `WALLET-CREATE`
- **And** the serialized body MUST contain exactly the canonical `from` owner and `to` factory wire fields used by `wallet_create_http_submit_request.json`

#### Scenario: Signed WALLET fixture uses canonical camelCase and decimal string fields

- **Given** an owner-signed WALLET batch request with one call
- **When** the client serializes the submit DTO for `POST /submit`
- **Then** the body MUST contain `type`, `from`, `to`, decimal-string `nonce`, `signature`, and `depositWalletParams`
- **And** `depositWalletParams` MUST contain `depositWallet`, decimal-string `deadline`, and `calls`
- **And** each call MUST contain `target`, decimal-string `value`, and `0x`-prefixed `data` as shown by `wallet_signed_http_submit_request.json`

#### Scenario: Submit responses require exact transactionID casing

- **Given** a relayer submit response object with `transactionID`, `state`, and an optional valid `transactionHash`
- **When** the client deserializes the submit response
- **Then** the response MAY become a transaction receipt using the exact `transactionID` and `transactionHash` keys
- **And** a response using `transactionId` instead of `transactionID`, a malformed `transactionID`, or a malformed `transactionHash` MUST be rejected or require reconciliation rather than being accepted as an equivalent wire shape

### Requirement: Group F - DepositWallet Batch EIP-712 typed data and digest

Deposit-wallet batch signing MUST build the implemented EIP-712 typed data before digesting or recovering signatures. The domain MUST use name `DepositWallet`, version `1`, the signed `chainId`, and `verifyingContract` equal to the deposit wallet. The primary type MUST be `Batch`. The implemented type graph/string MUST be `EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)`, `Call(address target,uint256 value,bytes data)`, and `Batch(address wallet,uint256 nonce,uint256 deadline,Call[] calls)Call(address target,uint256 value,bytes data)`. The EIP-712 message fields MUST be exactly `wallet`, `nonce`, `deadline`, and `calls`; call entries MUST contain `target`, `value`, and `data`. `nonce`, `deadline`, and call `value` MUST be represented as decimal strings in typed-data JSON. `owner`, `nonce_owner`, and `submit_from` are verification identities, not EIP-712 message fields.

#### Scenario: Typed data and digest match the official SDK fixture

- **Given** `wallet_batch_eip712.json` containing the implemented owner, deposit wallet, chain id, nonce, deadline, calls, expected typed data, expected digest, and owner signature
- **When** the client builds typed data and computes the DepositWallet Batch digest
- **Then** `primaryType` MUST be `Batch`
- **And** the domain MUST contain `DepositWallet`, version `1`, the fixture chain id, and `verifyingContract` equal to the fixture deposit wallet
- **And** the built typed data and digest MUST match the fixture values

#### Scenario: Domain or message mutation changes signature validity

- **Given** a signed batch whose fixture digest validates for its original domain and message
- **When** `chainId`, `verifyingContract`/wallet, `nonce`, `deadline`, call target, call value, call data, or call order is changed
- **Then** the digest MUST change or signature validation MUST fail
- **And** the client MUST NOT treat the original owner signature as valid for the mutated EIP-712 payload

#### Scenario: Verification identities are not encoded as EIP-712 message fields

- **Given** a batch carrying `owner`, `nonce_owner`, and `submit_from` for validation
- **When** typed data is built for signing
- **Then** the EIP-712 message MUST contain `wallet`, `nonce`, `deadline`, and `calls` only
- **And** `owner`, `nonce_owner`, and `submit_from` MUST remain verification identities outside the EIP-712 message field set

### Requirement: Group F - Signature shape, resource limits, and recovered owner signer

Deposit-wallet signature validation MUST accept only an ECDSA signature string shaped as `0x` plus 130 hex characters for a 65-byte signature. Typed-data building, digesting, signature recovery, and signature validation MUST enforce at most 256 calls and at most 1 MiB total call data. The recovered signer MUST match `owner` for a signed batch to validate. Non-owner, unauthorized, self-asserted session-signer, malformed, unrecoverable, over-call-limit, or over-calldata-limit signatures and payloads MUST be rejected rather than converted into wire DTOs.

#### Scenario: Owner signature validates and exposes the recovered owner

- **Given** `wallet_batch_eip712.json` with an `ownerSignature` and `ownerRecoveredSigner`
- **When** the client recovers the signer and validates the signed batch
- **Then** the recovered signer MUST match the fixture owner
- **And** the resulting signed batch MUST expose the verified signer as the owner

#### Scenario: Malformed signature shape is rejected

- **Given** a signature missing the `0x` prefix, shorter than `0x` plus 130 hex characters, longer than `0x` plus 130 hex characters, or containing non-hex characters
- **When** the client attempts signature recovery or signed-batch validation
- **Then** validation MUST fail with the implemented 0x-prefixed 65-byte signature requirement
- **And** no signed batch or WALLET request DTO MUST be produced from that malformed signature shape

#### Scenario: Calls limit exceeded is rejected

- **Given** a batch with 257 calls
- **When** the client builds typed data, computes a digest, recovers a signer, validates a signature, or uses the public signed WALLET builder
- **Then** the batch MUST be rejected as exceeding the maximum of 256 calls
- **And** the over-limit payload MUST NOT proceed to a WALLET submit request body

#### Scenario: Call data limit exceeded is rejected

- **Given** a batch whose total call data is greater than 1 MiB, either in one call or split across multiple calls
- **When** the client builds typed data, computes a digest, recovers a signer, validates a signature, or uses the public signed WALLET builder
- **Then** the batch MUST be rejected as exceeding the maximum call data bytes
- **And** a batch at exactly 1 MiB total call data MAY be accepted by the resource limit check

#### Scenario: Recovered signer mismatch is rejected

- **Given** a signature that recovers to a non-owner signer, unauthorized signer, or self-asserted session signer
- **When** the client validates the DepositWallet Batch signature
- **Then** validation MUST fail because the recovered signer does not match `owner`
- **And** the client MUST NOT accept the signature merely because it is recoverable and has the correct 65-byte shape

### Requirement: Group F - Signed DTO builders and deadline-clock boundary

`build_deposit_wallet_batch_request_from_signed` and `try_build_wallet_batch_request_with_signature` MUST validate signed input and build canonical WALLET wire DTOs, but they MUST NOT perform wall-clock deadline expiry checking. Builder validation MUST include signature shape, resource limits, signer recovery, `owner`/`nonce_owner`/`submit_from` identity equality, chain/config consistency, and deposit-wallet derivation from owner/config. `owner`, `nonce_owner`, and `submit_from` are verification identities, not EIP-712 message fields or independent wire fields. Live submit code MUST use its separate nonce-lease and deadline expiry guards before POST; the signed DTO builders alone MUST NOT be documented as a wall-clock expiry enforcement boundary.

#### Scenario: Signed builder returns the canonical WALLET fixture body

- **Given** a signed batch validated from `wallet_batch_eip712.json` and a matching deposit-wallet contract config
- **When** the client builds a WALLET request from the signed batch
- **Then** the serialized request MUST match `wallet_signed_submit_body.json`
- **And** the builder MUST preserve the canonical `type`, `from`, `to`, decimal-string `nonce`, `signature`, and `depositWalletParams` wire fields

#### Scenario: Identity mismatch is rejected before building the request

- **Given** a signed batch whose `nonce_owner` differs from `owner`, whose `submit_from` differs from `owner`, whose signed chain/config differs from the submit config, or whose deposit wallet differs from the owner/config derived wallet
- **When** the client validates or builds the signed WALLET request
- **Then** validation MUST fail with a signing error
- **And** the client MUST NOT serialize or submit a WALLET body for the mismatched identity or config

#### Scenario: Deadline-clock misconception is rejected

- **Given** a signed batch whose signature, resource limits, identities, chain/config, and derived wallet are otherwise valid
- **When** `build_deposit_wallet_batch_request_from_signed` or `try_build_wallet_batch_request_with_signature` builds the request
- **Then** the builder MUST NOT perform wall-clock deadline expiry checking
- **And** a spec or caller MUST NOT claim that these builders reject an already expired deadline by consulting the current clock
- **And** when the same signed batch flows through live `sign_and_submit_wallet_batch_with_nonce_lease`, the separate nonce-lease expiry check and signed-batch deadline guard MUST reject expired inputs before `POST /submit`

<!-- GROUP-B: owner mutation state, ambiguous submit, and reconciliation -->

### Requirement: Group B - Client-local owner mutation coordination

The deposit-wallet relayer client MUST coordinate owner-scoped mutation state inside the current client allocation. Cloned clients share the same in-memory owner mutation store, but separately constructed clients and process restarts MUST NOT be treated as coordinated or durable. For each owner, an `InFlight` block, an `Ambiguous` block, or an active nonce-read reservation MUST block later mutation or nonce-lease work for that same owner before an additional owner-scoped HTTP request is sent. Independent owners MAY proceed independently when their own owner state is unblocked. The internal shard count is design trace only and MUST NOT be part of the external contract.

#### Scenario: Same-owner in-flight submit blocks duplicate mutation before another HTTP request

- **Given** an owner has an in-flight submit reservation or in-flight transaction record in the client-local owner mutation store
- **When** the caller attempts another WALLET-CREATE submit, WALLET batch submit, or WALLET nonce-lease read for the same owner
- **Then** the client MUST return a reconciliation-required or mutation-blocked owner-state error before sending another owner-scoped HTTP request
- **And** the error MUST direct the caller to wait, poll to a terminal state, or reconcile before another owner mutation

#### Scenario: Same-owner ambiguous submit blocks duplicate mutation until reconciliation

- **Given** an owner has an ambiguous submit block with a recorded payload hash
- **When** the caller attempts another WALLET-CREATE submit, WALLET batch submit, or WALLET nonce-lease read for that owner
- **Then** the client MUST reject the work before another owner-scoped HTTP request
- **And** the ambiguous owner MUST remain blocked until implemented reconciliation rules clear it

#### Scenario: Independent owner state does not block another owner

- **Given** owner A has an in-flight or recorded submit state in a shared cloned client allocation
- **When** owner B submits with a valid permit and no owner B block
- **Then** owner B MAY proceed through the submit path and send its own request
- **And** owner A's block MUST NOT be treated as a global client-wide mutation lock

#### Scenario: Durable-state and shard-count misconceptions are rejected

- **Given** a caller relies on process restart, a separately constructed client, another machine, or an exact internal shard count to coordinate owner mutation state
- **When** the caller reasons about duplicate-submit prevention
- **Then** the external contract MUST be only client-local, non-durable, owner-scoped mutation coordination
- **And** any internal `64 shards` implementation detail MUST remain a non-normative design trace, not a consumer-visible durability or routing guarantee

### Requirement: Group B - Ambiguous submit transitions and idempotency evidence

After a submit reservation reaches the POST boundary, the client MUST treat failures whose relayer acceptance is unknown as ambiguous owner outcomes. Dropping an armed reservation, receiving a timeout or transport failure, receiving a non-pre-acceptance API failure, receiving an oversized success body, receiving an unparseable or partial success body, receiving a success body without a usable `transactionID`, or encountering an unclassified post-boundary failure MUST record an owner ambiguous block and return `ambiguous_submit` or reconciliation-required behavior as implemented. The ambiguous block MUST be keyed by owner and by a payload hash summary of the submitted body or signed digest evidence, not by raw request bodies or secrets. Supported pre-acceptance failures such as auth-header failure and the implemented `401`, `403`, and `404` API responses MUST clear the reservation instead of leaving the owner ambiguous.

#### Scenario: Dropped armed submit leaves the owner ambiguous

- **Given** a submit reservation has been armed for ambiguous-on-drop immediately before `POST /submit`
- **When** the submit future is dropped after the POST boundary and before a trusted response is processed
- **Then** the owner block MUST transition from in-flight to ambiguous for the same payload hash
- **And** later same-owner mutation or nonce-lease work MUST be blocked before another HTTP request

#### Scenario: Timeout, transport, API, quota, oversized, and unclassified post-boundary failures are ambiguous

- **Given** an owner submit has crossed the POST boundary
- **When** the client observes a timeout, connect/body/decode/request/transport error, HTTP status other than the pre-acceptance clear set, quota exhaustion, an oversized success body, or an unclassified post-boundary error
- **Then** the client MUST record the owner payload as ambiguous
- **And** it MUST return an `ambiguous_submit` error that requires owner-scoped manual reconciliation before duplicate submit

#### Scenario: Partial or transactionID-less success responses keep the owner ambiguous

- **Given** a `POST /submit` success response is unusable, uses the wrong `transactionID` casing, has a blank or malformed `transactionID`, is an array where a submit object is expected, or otherwise lacks a usable transaction id
- **When** the submit response parser cannot produce a trusted receipt
- **Then** the client MUST record an ambiguous block for the owner payload without trusting the partial response as a transaction owner record
- **And** same-owner mutation MUST remain blocked until idless or transaction-id reconciliation succeeds under the implemented rules

#### Scenario: Partial success response with a usable transactionID records transaction-id evidence

- **Given** a success response includes a syntactically usable `transactionID` but is otherwise unusable or reaches an implemented reconciliation-required submit state
- **When** the client handles the post-boundary response
- **Then** it MUST record that transaction id as recorded or unrecorded evidence for the owner payload according to local state availability
- **And** the owner ambiguous block MUST require transaction-id reconciliation or trusted terminal observation before release

#### Scenario: Pre-acceptance failures clear the reservation instead of making the owner ambiguous

- **Given** submit work fails before relayer acceptance because relayer auth header construction fails or the relayer returns one of the implemented pre-acceptance `401`, `403`, or `404` statuses
- **When** the submit path returns that error
- **Then** the client MUST clear the owner reservation
- **And** the owner MUST NOT be left with an ambiguous block from that pre-acceptance failure alone

### Requirement: Group B - Transaction-id manual reconciliation validates evidence before release

Transaction-id manual reconciliation MUST use a `ManualReconciliation` mutation permit and `DepositWalletSubmitReconciliationEvidence` for the same owner, same client reconciliation scope, same issuer as the permit, and the same payload hash as the current ambiguous owner block. Its observation MUST contain a valid transaction id, a non-empty reason, a non-zero check timestamp that does not predate the ambiguous block and is not too far in the future, and a terminal observed state. `Confirmed` evidence MUST include a valid transaction hash; `Invalid` and `Failed` evidence MUST be terminal failure observations and MUST discard any supplied hash; non-terminal or `Unknown` observations MUST be rejected. The ambiguous block MUST be released only after local transaction records or observed unrecorded transaction ids match the owner payload and any trusted terminal observation matches the submitted evidence.

#### Scenario: Matching transaction-id evidence clears a fully reconciled ambiguous owner block

- **Given** an owner has an ambiguous block for payload hash P and a local or observed transaction id for P
- **And** a `ManualReconciliation` permit and transaction-id evidence match the owner, scope, issuer, payload hash, transaction id, terminal state, and trusted terminal observation when one exists
- **When** the caller clears the ambiguous submit with transaction-id reconciliation
- **Then** the client MAY remove the matched transaction record or unrecorded transaction id
- **And** it MUST release the owner ambiguous block only when no additional local or observed transaction ids remain for that owner payload

#### Scenario: Mismatched payload, owner record, or trusted observation does not partially clear

- **Given** an ambiguous owner block has payload hash P or a trusted terminal observation for transaction T
- **When** reconciliation evidence uses a different payload hash, matches a transaction record for another owner or payload, has no matching local owner payload record when one is required, or does not match the trusted terminal observation
- **Then** the client MUST return reconciliation-required or mutation-blocked according to the failed validation
- **And** it MUST keep the ambiguous owner block and remaining transaction-id evidence intact

#### Scenario: Non-terminal, unknown, stale, future, or confirmed-without-hash evidence is rejected

- **Given** transaction-id reconciliation evidence has `New`, `Executed`, `Mined`, `Unknown`, `Confirmed` without a transaction hash, a missing reason, a zero check timestamp, a check timestamp before the ambiguous block, or a check timestamp beyond the allowed future skew
- **When** the caller constructs or applies that evidence
- **Then** the client MUST reject the evidence before treating it as a successful reconciliation
- **And** the owner ambiguous block MUST remain until valid terminal evidence is supplied

#### Scenario: Multiple transaction ids are reconciled one at a time

- **Given** an ambiguous owner payload has more than one recorded or observed transaction id
- **When** the caller reconciles only one transaction id with valid terminal evidence
- **Then** the client MUST remove or acknowledge only that transaction id and return reconciliation-required for the remaining ids
- **And** the owner ambiguous block MUST be released only after every recorded or observed transaction id for the payload has been reconciled

### Requirement: Group B - Idless manual reconciliation is limited to recordless ambiguous payloads

Idless manual reconciliation MUST use a `ManualReconciliation` mutation permit and `DepositWalletIdlessSubmitReconciliationEvidence` for the same owner, same client reconciliation scope, same issuer as the permit, and the same payload hash as the current ambiguous owner block. Idless evidence is an audited manual assertion, with non-empty reason and check timestamp, that no accepted transaction was found for the ambiguous payload. It MUST be able to release only a recordless ambiguous block with no local transaction records and no observed unrecorded transaction ids for that owner payload. If local records or observed ids exist, transaction-id reconciliation MUST be required instead. If no owner block exists, the implemented clear operation MAY be idempotent and leave state clear.

#### Scenario: Idless evidence clears a recordless ambiguous owner block

- **Given** an owner has an ambiguous block for payload hash P and no local transaction records or observed unrecorded transaction ids for P
- **And** the idless evidence has `ManualReconciliation` scope, matching issuer, matching payload hash, a non-empty reason, and a valid check timestamp
- **When** the caller clears the ambiguous submit with idless reconciliation
- **Then** the client MAY remove the ambiguous owner block
- **And** subsequent owner mutation work is no longer blocked by that recordless ambiguous payload

#### Scenario: Idless evidence is rejected when transaction evidence exists

- **Given** the ambiguous owner payload has a local transaction record, an observed unrecorded transaction id, or a transaction id that could not be associated because of a local record conflict
- **When** the caller attempts idless reconciliation for that payload
- **Then** the client MUST return reconciliation-required
- **And** the owner ambiguous block MUST remain until transaction-id reconciliation handles the observed transaction evidence

#### Scenario: Idless evidence rejects mismatched, stale, or future audit evidence

- **Given** idless evidence has a mismatched payload hash, a non-`ManualReconciliation` scope, an issuer different from the permit issuer, an empty reason, a zero check timestamp, a check timestamp before the ambiguous block, or a check timestamp beyond the allowed future skew
- **When** the caller constructs or applies that evidence
- **Then** the client MUST reject the idless evidence
- **And** the ambiguous owner block MUST remain unless the implemented no-block idempotent path applies

#### Scenario: Idless clear without an owner block is idempotent only for already-clear local state

- **Given** the client-local owner mutation store has no ambiguous block for the owner
- **When** the caller applies otherwise well-formed idless reconciliation evidence
- **Then** the implemented operation MAY return success without changing state
- **And** callers MUST NOT treat that as proof of durable cross-client or cross-process reconciliation

### Requirement: Group B - Terminal observations clear only trusted matching in-flight records

Terminal transaction observations MUST update client-local owner mutation state according to the current owner block and transaction evidence. A trusted terminal receipt for a matching in-flight owner, transaction id, payload hash, and transaction type MUST remove the transaction record and clear the owner block when no other records remain. `Confirmed` terminal observations MUST include a valid transaction hash; `Invalid` and `Failed` are terminal failures; `New`, `Executed`, and `Mined` are non-terminal and MUST leave the in-flight block in place; `Unknown`, missing owner proof, missing confirmed hash, or transaction type mismatch MUST require reconciliation and may transition matching in-flight state to ambiguous. For an already ambiguous owner, terminal observations MUST be stored as trusted observations for later manual reconciliation rather than clearing the ambiguous block or returning success immediately.

#### Scenario: Confirmed or failed terminal transaction read clears a matching in-flight owner record

- **Given** an owner has an in-flight transaction record from a previous accepted submit
- **When** `get_transaction_for_owner` observes a matching terminal receipt with `Confirmed` plus a valid transaction hash, or observes `Failed` or `Invalid` terminal failure for the same owner payload and transaction type
- **Then** the client MUST remove the matching transaction record
- **And** it MUST clear the owner block when no other transaction records remain for that payload

#### Scenario: Pending observations keep the in-flight block

- **Given** an owner has an in-flight transaction record
- **When** `get_transaction_for_owner` observes `New`, `Executed`, or `Mined` for the matching transaction
- **Then** the client MUST return transaction-absent pending behavior
- **And** the owner in-flight block MUST continue to prevent another same-owner mutation before terminal observation

#### Scenario: Reconciliation-required observations transition matching in-flight records to ambiguous

- **Given** an owner has an in-flight transaction record
- **When** `get_transaction_for_owner` observes `Unknown`, `Confirmed` without a valid transaction hash, missing owner evidence, owner mismatch, or transaction type mismatch
- **Then** the client MUST require reconciliation and transition the matching in-flight record to ambiguous where implemented
- **And** same-owner mutation MUST remain blocked before duplicate submit

#### Scenario: Ambiguous records store trusted terminal observations until manual reconciliation

- **Given** an owner already has an ambiguous block for a payload and a transaction id associated with that payload
- **When** `get_transaction_for_owner` observes a terminal receipt for that transaction
- **Then** the client MUST store the trusted terminal observation for later evidence matching
- **And** it MUST NOT clear the ambiguous block or return a confirmed success until manual reconciliation releases the owner block

### Requirement: Group B - Owner-state evidence remains redacted and secret-free

Owner-state, ambiguous-submit, and reconciliation evidence MUST use local owner addresses, transaction ids, transaction hashes, and payload hashes only for mutation coordination and manual reconciliation. The recorded idempotency value MUST be a payload hash summary or signed-digest hash, not the raw submit body, raw EIP-712 signature input, private key, or relayer API key. Error and debug paths for Group B evidence MUST use the implemented redacted address, sanitized external token, shortened payload hash, and redacted issuer/reason formatting where those values are surfaced. This requirement does not add durable storage or a new wallet-address disclosure contract beyond the already implemented owner, transaction, and payload evidence paths.

#### Scenario: Ambiguous errors expose summaries instead of raw submit secrets

- **Given** a post-boundary submit outcome is ambiguous for an owner payload
- **When** the client records and reports the ambiguous owner block
- **Then** the owner block MUST store the payload hash summary used for reconciliation
- **And** error text MUST render owner addresses, transaction ids, and payload hashes through redacted or summarized forms rather than raw request bodies, private keys, relayer API keys, or raw signed payload material

#### Scenario: Reconciliation debug output redacts issuer and reason fields

- **Given** transaction-id or idless reconciliation evidence contains issuer, reason, owner, transaction id, transaction hash, and payload hash data
- **When** debug output is formatted for that evidence or its observation
- **Then** issuer and reason MUST be redacted
- **And** owner, transaction id, transaction hash, and payload hash MUST be surfaced only through the implemented redaction or summary helpers

#### Scenario: Raw durable wallet-state assumptions are rejected

- **Given** a consumer expects Group B local evidence to be a durable wallet-state ledger or to expose raw wallet/secret material for external reconciliation
- **When** the consumer uses the Group B contract
- **Then** the contract MUST remain limited to client-local owner mutation coordination and manual reconciliation evidence
- **And** it MUST NOT promise raw secret exposure, durable wallet-state persistence, or cross-client reconciliation storage

