# Royalty Splitter (Soroban)

A trustless revenue splitter for content platforms. It collects an incoming
subscription payment and atomically routes it to `N` payees according to
integer percentage shares — with **no fractional dust stranded in the
contract**.

## Guarantees

- **Exact conservation.** The configured shares always sum to `1_000_000`
  (parts per million, i.e. six decimals of precision). The first `N - 1` payees
  receive the truncated share and the **final payee receives the remainder**, so
  the allocations sum to exactly the payment amount and the contract balance
  returns to zero after every call.
- **Atomic settlement.** `process_payment` pulls the payment in and fans it out
  inside a single invocation. If any transfer fails the whole call reverts, so
  funds can never be stuck mid-distribution.
- **Unanimous reconfiguration.** Changing payees requires an `approvals` list
  naming every *current* payee, and each of those payees must authorize the
  call. Existing stakeholders therefore hold a collective veto.
- **Standard token compatibility.** Value moves only through the canonical
  Soroban token interface, so the contract works with any Stellar Asset
  Contract (SAC) or SAC-compatible token.

## Interface

| Function | Description |
| --- | --- |
| `initialize(payees)` | One-shot setup. Every founding payee must authorize the call. |
| `process_payment(from, asset, amount)` | Pull `amount` of `asset` from `from` and route it to all payees. |
| `update_splits(new_payees, approvals)` | Unanimous multi-sig reconfiguration of the split. |
| `get_config() -> SplitConfig` | Active payee set. |
| `preview(amount) -> Vec<Allocation>` | Resolve a payment into exact payouts without moving funds. |
| `payments_processed() -> u64` | Number of settled payments. |
| `total_routed() -> i128` | Cumulative amount routed to payees. |

### Types

```rust
struct Payee { address: Address, shares: u32 }   // shares out of 1_000_000
struct SplitConfig { payees: Vec<Payee> }         // order matters; last absorbs dust
struct Allocation { address: Address, amount: i128 }
```

### Errors

`AlreadyInitialized`, `NotInitialized`, `EmptyPayees`, `TooManyPayees`,
`DuplicatePayee`, `ZeroShare`, `SharesDoNotSumToTotal`, `InvalidAmount`,
`MathOverflow`, `MissingApproval`.

## Worked example

Split `10_000` tokens `33.333% / 33.333% / 33.334%`
(`333_330 / 333_330 / 333_340` shares):

| Payee | Entitlement | Truncated | Paid |
| --- | --- | --- | --- |
| 1 | 3 333.3 | 3 333 | **3 333** |
| 2 | 3 333.3 | 3 333 | **3 333** |
| 3 (last) | 3 333.4 | 3 333 | **3 334** (its share + 1 unit of dust) |

Total paid: `10 000`. Contract balance: `0`.

## Building & testing

This crate is a standalone Cargo workspace so it does not affect the parent
`stellar-k8s` operator build.

```bash
cd contracts/royalty-splitter
cargo test                       # unit tests in src/ + tests/integration.rs
cargo build --release \
  --target wasm32-unknown-unknown  # produces target/.../royalty_splitter.wasm
```

The integration suite deploys a real Stellar Asset Contract
(`register_stellar_asset_contract_v2`) and asserts, among other things, the
exact dust-free `3_333 / 3_333 / 3_334` distribution above.
