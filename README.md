# Rust Exchange Similator in Rust with gRPC

## Configuration
Configuration in .env file

## Tests
```shell
export RUST_LOG=info
cargo run --bin server
# tests expecting running instance on :50051
cargo test -- --nocapture
```

## Build (development)

### Server
```shell
# set the right level of logging
export RUST_LOG=info
cargo run --bin server
```

### Client
```shell
# set the right level of logging
export RUST_LOG=info
cargo run --bin client
```

## Build (release)
```shell
cargo build --release
```

## Run
```shell
#server
./target/release/server
#client
./target/release/client
```

## Architeture decisions
- HashMap performance is O(1), while BTreeMap performance is O(log N), however we have just 2 keys and doing a lot insert/delete/lookup where HashMap should be better.
- Ordered_float crate in Rust that provides a way to handle f64 and f32 floating-point numbers with total ordering. The standard f64 and f32 types in Rust do not implement the Ord trait because floating-point numbers do not have a total order due to the presence of special values like NaN (Not a Number). OrderedFloat solves this problem by providing a total order for floating-point numbers.
