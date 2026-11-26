# Parallel Database Reads

Independent `SELECT` queries outside a transaction may run concurrently on the pool with `tokio::try_join!`. Follow the paginated-list pattern:

```rust
let (rows, total) = tokio::try_join!(
    rows_query.fetch_all(&self.pool),
    count_query.fetch_one(&self.pool),
)?;
```

**Do not** run queries concurrently inside a transaction: a `Transaction` is one connection, so there is no benefit (and sqlx needs `&mut` access for each query).

**Do not** parallelize writes without a transaction.

## Reads Before Transactions

```text
Good:  parallel reads on the pool  ->  transaction with writes only
Bad:   transaction with parallel reads + writes
```

Prefetch independent reads outside the transaction. Read inside the transaction only when the read must share the snapshot of the writes.

## Pool Size

`DB_POOL_MAX` is **4** (`packages/database/src/env.rs`). Do not fan out more concurrent queries than the pool holds; for larger fan-outs, bound concurrency (for example `futures::stream::iter(...).buffer_unordered(4)`) instead of an unbounded join.
