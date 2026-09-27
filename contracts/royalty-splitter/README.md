# Royalty Splitter (Soroban)

A Soroban smart contract that intercepts incoming subscription payments and
routes fractional percentages to a configurable set of revenue stakeholders
(creators, platform, affiliates) with **zero stranded dust**.

- **Language / SDK:** Rust, `soroban-sdk` v28
- **Target:** `wasm32v1-none` (built with `stellar contract build`)
- **Entrypoints:** `initialize`, `process_payment`, `propose_splits`,
  `approve_splits`, `cancel_splits`, `get_splits`, `get_token`, `get_admin`,
  `get_pending`

## Split model

Percentages are stored as integer **parts-per-million** (`share_ppm`), where
`SCALE == 1_000_000` equals 100%. This gives four decimal places of a percent,
which is required to express values a basis-point (`10_000`) model cannot — for
example `33.333% == 333_330` ppm. Configurations must be non-empty, use strictly
positive weights, contain no duplicate payee addresses, and sum to exactly
`SCALE`.

### Dust-free distribution

For a payment of `amount`:

1. Each non-final payee receives `floor(amount * share_ppm / SCALE)`.
2. The final payee receives `amount - sum(leading_shares)`.

Because every leading share is floored, the remainder is always in
`[0, payee_count)` and is handed to the last payee, so `sum(shares) == amount`
holds exactly. The classic off-by-one case is covered directly:

| Payee | Weight (`share_ppm`) | Exact share | Routed |
| ----- | -------------------- | ----------- | ------ |
| A     | 333_330 (33.333%)    | 3,333.30    | 3,333  |
| B     | 333_330 (33.333%)    | 3,333.30    | 3,333  |
| C     | 333_340 (33.334%)    | 3,333.40    | 3,334  |
|       | **1,000,000 (100%)** | **10,000**  | **10,000** |

The arithmetic splits `amount` around `SCALE` before multiplying, so no
intermediate product can overflow `i128` even for balances near the maximum
token amount.

## Payment flow

`process_payment(payer, amount)` requires `payer` authorization and executes one
token transfer per payee, sequentially, against the SEP-41 / Stellar Asset
Contract configured at initialization. The contract never holds funds, so no
dust can be trapped in it. Zero-sized shares (possible for very small payments)
are skipped because the token contract rejects zero-value transfers.

## Reconfiguration (multi-sig)

Splits are dynamic but cannot be changed unilaterally:

1. `propose_splits(proposer, new_splits)` — the proposer must be the admin or a
   current stakeholder. The proposal is stored and changes nothing yet.
2. `approve_splits(approver)` — each **current** stakeholder approves once. When
   the last current stakeholder approves, the new table is applied
   automatically. Approvals are scoped to the current stakeholder set, so
   stakeholders cannot be swapped out mid-proposal.
3. `cancel_splits(caller)` — the proposer or admin can discard a pending
   proposal.

## Layout

```
contracts/royalty-splitter/
├── src/
│   ├── lib.rs           # contract entrypoints, state, events
│   └── distribution.rs  # pure split math + configuration validation
└── tests/
    └── integration.rs   # end-to-end tests against a real Stellar Asset Contract
```

## Build & test

Unit tests (split math/validation) and integration tests (real SAC transfers)
need only a host toolchain:

```bash
cd contracts/royalty-splitter
cargo test
cargo clippy --all-targets -- -D warnings
```

The deployable Wasm requires the [Stellar CLI](https://github.com/stellar/stellar-cli)
v25.2.0+ and the `wasm32v1-none` target:

```bash
rustup target add wasm32v1-none
stellar contract build
# → target/wasm32v1-none/release/royalty_splitter.wasm
```

Alternatively, from the repository root:

```bash
make contracts-test
```

## Security notes

- Every state-changing call requires `Address::require_auth` on the relevant
  account (`admin`, `payer`, `approver`, `proposer`/`caller`).
- A proposal can never be applied without the approval of **every** current
  stakeholder.
- Distribution is deterministic integer math; there is no floating point and no
  rounding direction that can create or destroy value.
