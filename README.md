# Rust Exchange Similator in Rust with gRPC

## Description
This simple Rust Exchange provides an example of implementation of a cryptocurrency orderbook with live connectivity to real crypto exchange. It has also option to run in --offline mode and use predefined orderbook to support testing and e.g., UI build. It supports real-time order matching for market and limit orders. Designed for performance and scalability, this project leverages Rust's memory safety and concurrency features to ensure high reliability and low latency. Key features include:

Final version will:
- Order Matching Engine: Handles various order types (limit, market) with high precision and speed from gRPC client CLI.
- Real-time Updates: Ensures immediate reflection of order book changes and market data.
- Concurrency: Utilizes Rust's async capabilities to process multiple transactions simultaneously.
- Ideal for developers looking to build high-performance trading systems and applications in the cryptocurrency space.

Current version:
- gRPC-based server
    - with periodic orderbook update from Kraken exchange using public API
    - orderbook data stored in in-memory cache with persistency to a disk
    - trade matching engine with logic to process market and limit order only (stop/cancel in progress)
- gRPC-based client
    - with sections for:
        - price updates
        - orderbook preview
        - trade execution
        - tradebook preview (in progress)
        - support to tradebook per trader (in progress)

## Configuration

Configuration in .env file (not required now)

## Tests
```shell
export RUST_LOG=info
cargo run --bin server
# tests expecting running instance on :50051
cargo test -- --nocapture
```

## Build
```shell
cargo build --release
```

## Run (development)

### Server (development)
```shell
# set the right level of logging
export RUST_LOG=info
cargo run --bin server
```

### Client (development)
```shell
# set the right level of logging
export RUST_LOG=info
cargo run --bin client market-order XXBTZUSD 1.4 buy market 0.0 Jan
cargo run --bin client market-order XXBTZUSD 1.4 sell market 0.0 Jan
cargo run --bin client market-order XXBTZUSD 1.4 buy limit 65248.0 Jan
cargo run --bin client market-order XXBTZUSD 1.4 sell limit 65248.0 Jan
```

## Architeture decisions
- HashMap performance is O(1), while BTreeMap performance is O(log N), however we have just 2 keys and doing a lot insert/delete/lookup where HashMap should be better.
- Ordered_float crate in Rust that provides a way to handle f64 and f32 floating-point numbers with total ordering. The standard f64 and f32 types in Rust do not implement the Ord trait because floating-point numbers do not have a total order due to the presence of special values like NaN (Not a Number). OrderedFloat solves this problem by providing a total order for floating-point numbers.

## Notes

### Trade status
- New: The order has been received by the exchange but has not yet been processed or entered into the order book.
- Pending: The order is under review or awaiting certain conditions before it can be entered into the order book.
- Open: The order is active and has been entered into the order book. It is waiting to be matched with a counter order.
- Partially Filled: Part of the order has been matched and executed, but the entire order is not yet fully completed. The remaining portion remains open in the order book.
- Filled: The entire order has been matched and executed. There is no remaining quantity left in the order.
- Canceled: The order has been canceled by the trader or broker and will not be executed. This can happen before the order is fully or partially filled.
- Rejected: The order has been rejected by the exchange, possibly due to insufficient funds, incorrect order details, or violation of trading rules.
- Expired: The order has expired based on the time conditions set (e.g., good for day orders that are not filled by the end of the trading day).
- Pending Cancel: A cancellation request has been submitted for the order, but it has not yet been confirmed or processed.
- Pending Replace: A modification request has been submitted for the order (e.g., change in quantity or price), but it has not yet been confirmed or processed.

### Trade Status Flow
New -> Pending -> Open
Open -> Partially Filled -> Filled
Open -> Canceled
Pending -> Rejected
Open -> Expired
