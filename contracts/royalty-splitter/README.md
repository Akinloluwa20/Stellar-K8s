# Royalty Splitter Contract

A dynamic royalty splitter for content platforms: incoming subscription payments
are automatically routed to N payees according to configurable percentage
splits, **without ever stranding fractional token dust**.

Built with [soroban-sdk](https://crates.io/crates/soroban-sdk) 21 (Stellar
Protocol 21).

## Features

- **N-payee configuration engine** — payees are registered with basis-point
  splits (1 bp = 0.01%) that must sum to exactly 10,000 (100%).
- **Dust-free distribution** — each payee receives the floor of their
  percentage of a payment, and the fractional remainder (dust) is assigned
  explicitly to the final payee in the list. The sum of payouts always equals
  the payment amount exactly.
- **High-precision math** — all arithmetic is integer basis-point math over
  `i128`; no floating point anywhere in the calculation path.
- **Multisig reconfiguration** — changing splits or payees requires an
  ed25519-signed approval from *every* current stakeholder. The new
  configuration commits atomically when the last signature arrives.

## Layout

| File | Purpose |
| --- | --- |
| `src/lib.rs` | Contract entry points, storage, authorization, multisig flow |
| `src/distribution.rs` | Pure split math (validations, payout calculation) |
| `src/test.rs` | Integration tests against the Stellar Asset Contract token |

## Contract interface

| Function | Description |
| --- | --- |
| `initialize(token, payees, splits, payee_keys)` | One-time setup: token, payees (order matters — the final payee receives dust), basis-point splits, and ed25519 keys for approvals. |
| `process_payment(payer, amount)` | Pulls `amount` of the configured token from `payer` and routes floor-truncated shares to every payee sequentially. Leaves zero balance in the contract. |
| `propose_reconfig(proposer, payees, splits, keys)` | Stakeholder-only: puts a new configuration up for multisig approval. |
| `approve_reconfig(payee, signature)` | Records an ed25519-signed approval; commits when all stakeholders have signed. |
| `cancel_reconfig(caller)` | Stakeholder-only: discards a pending proposal. |
| `get_config` / `get_pending_config` / `get_approvals` / `get_token` | Read-only introspection. |
| `simulate_payouts(amount)` | Returns the exact payout vector for `amount` without moving funds. |

## Example

Send 10,000 tokens through a `33.333% / 33.333% / 33.334%` split
(`[3_333, 3_333, 3_334]` basis points):

| Payee | Share |
| --- | --- |
| Payee 1 | 3,333 |
| Payee 2 | 3,333 |
| Payee 3 (final) | 3,334 |
| **Total** | **10,000** (exact, zero dust) |

## Building

```sh
# Host build and tests
cargo test

# Deployable WASM artifact
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
# → target/wasm32-unknown-unknown/release/royalty_splitter.wasm
```

## Testing

The test suite covers:

- The spec validation case: 10,000 tokens through a 33.333/33.333/33.334 split
  with exact per-payee assertions.
- A 1,000-amount sweep proving no dust is stranded for any payment size.
- Uneven amounts, individually truncating splits, and four-payee layouts.
- Configuration validation (sum ≠ 100%, out-of-range splits, duplicates).
- Multisig reconfiguration: partial approval does not commit, full approval
  commits atomically, wrong signers and duplicate approvals are rejected,
  and stakeholders can cancel stale proposals.

Integration tests run against the SDK's Stellar Asset Contract (SAC) token —
the same token interface standard issued assets expose on testnet/mainnet —
verifying compatibility with standard Soroban asset transfers.
